//! Lightweight package-manifest discovery for agent context.
//! No package manager execution, network access, lockfile resolution or dependency installation.
use crate::knowledge::{io, validate_repo, Result, MAX_FILE_BYTES};
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Dependency {
    pub repo: String,
    pub manifest: String,
    pub ecosystem: String,
    pub package: String,
    pub requirement: String,
    pub scope: String,
    pub local_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Package {
    pub repo: String,
    pub manifest: String,
    pub ecosystem: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub packages: Vec<Package>,
    pub dependencies: Vec<Dependency>,
    pub local_links: usize,
    pub scanned_manifests: usize,
    pub limitations: Vec<String>,
}

pub fn scan(roots: &BTreeMap<String, PathBuf>) -> Result<Report> {
    if roots.is_empty() || roots.len() > 32 { return Err("manifest scan requires 1..32 repository roots".into()); }
    let mut packages = Vec::new();
    let mut dependencies = Vec::new();
    let mut manifests = 0usize;
    for (repo, root) in roots {
        validate_repo(repo)?;
        let root = root.canonicalize()?;
        let mut seen = 0usize;
        for entry in ignore::WalkBuilder::new(&root)
            .hidden(false)
            .add_custom_ignore_filename(".codannaignore")
            .follow_links(false)
            .build()
        {
            let entry = entry?;
            if !entry.file_type().is_some_and(|t| t.is_file()) { continue; }
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue; };
            if !is_manifest(name) { continue; }
            seen += 1;
            manifests += 1;
            if seen > 256 || manifests > 2048 { return Err("manifest scan limit exceeded".into()); }
            let rel = path.strip_prefix(&root)?.to_str().ok_or("non UTF-8 manifest path")?.replace('\\', "/");
            let text = String::from_utf8(io::read_bounded(path, MAX_FILE_BYTES as u64)?)?;
            parse_manifest(repo, &rel, name, &text, &mut packages, &mut dependencies)?;
        }
    }

    let providers: BTreeMap<String, String> = packages.iter().filter_map(|p| p.name.as_ref().map(|name| (name.clone(), p.repo.clone()))).collect();
    let mut local_links = 0usize;
    for dep in &mut dependencies {
        if let Some(provider) = providers.get(&dep.package) {
            if provider != &dep.repo {
                dep.local_provider = Some(provider.clone());
                local_links += 1;
            }
        }
    }
    packages.sort_by(|a, b| (&a.repo, &a.manifest).cmp(&(&b.repo, &b.manifest)));
    dependencies.sort_by(|a, b| (&a.repo, &a.manifest, &a.scope, &a.package).cmp(&(&b.repo, &b.manifest, &b.scope, &b.package)));
    Ok(Report {
        packages,
        dependencies,
        local_links,
        scanned_manifests: manifests,
        limitations: vec![
            "Manifest parsing is static and intentionally shallow; lockfiles, conditional resolution, aliases and workspace inheritance may change the effective dependency graph.".into(),
            "No package manager, build script, plugin or network request is executed.".into(),
        ],
    })
}

fn is_manifest(name: &str) -> bool {
    matches!(name, "package.json" | "composer.json" | "Cargo.toml" | "pyproject.toml" | "go.mod")
}

fn parse_manifest(repo: &str, path: &str, name: &str, text: &str, packages: &mut Vec<Package>, dependencies: &mut Vec<Dependency>) -> Result<()> {
    match name {
        "package.json" => parse_json(repo, path, "npm", text, &["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"], packages, dependencies),
        "composer.json" => parse_json(repo, path, "composer", text, &["require", "require-dev"], packages, dependencies),
        "Cargo.toml" => parse_toml(repo, path, "cargo", text, &["dependencies", "dev-dependencies", "build-dependencies"], packages, dependencies),
        "pyproject.toml" => parse_pyproject(repo, path, text, packages, dependencies),
        "go.mod" => parse_go(repo, path, text, packages, dependencies),
        _ => Ok(()),
    }
}

fn parse_json(repo: &str, path: &str, ecosystem: &str, text: &str, sections: &[&str], packages: &mut Vec<Package>, dependencies: &mut Vec<Dependency>) -> Result<()> {
    let value: JsonValue = serde_json::from_str(text)?;
    let package_name = value.get("name").and_then(JsonValue::as_str).map(str::to_owned);
    packages.push(Package { repo: repo.into(), manifest: path.into(), ecosystem: ecosystem.into(), name: package_name });
    for section in sections {
        if let Some(items) = value.get(*section).and_then(JsonValue::as_object) {
            for (package, requirement) in items {
                dependencies.push(Dependency { repo: repo.into(), manifest: path.into(), ecosystem: ecosystem.into(), package: package.clone(), requirement: requirement.as_str().unwrap_or("<structured>").into(), scope: (*section).into(), local_provider: None });
            }
        }
    }
    Ok(())
}

fn parse_toml(repo: &str, path: &str, ecosystem: &str, text: &str, sections: &[&str], packages: &mut Vec<Package>, dependencies: &mut Vec<Dependency>) -> Result<()> {
    let value: toml::Value = toml::from_str(text)?;
    let package_name = value.get("package").and_then(|v| v.get("name")).and_then(toml::Value::as_str).map(str::to_owned);
    packages.push(Package { repo: repo.into(), manifest: path.into(), ecosystem: ecosystem.into(), name: package_name });
    for section in sections {
        if let Some(items) = value.get(*section).and_then(toml::Value::as_table) {
            for (package, requirement) in items {
                dependencies.push(Dependency { repo: repo.into(), manifest: path.into(), ecosystem: ecosystem.into(), package: package.clone(), requirement: requirement.as_str().map(str::to_owned).unwrap_or_else(|| requirement.to_string()), scope: (*section).into(), local_provider: None });
            }
        }
    }
    Ok(())
}

fn parse_pyproject(repo: &str, path: &str, text: &str, packages: &mut Vec<Package>, dependencies: &mut Vec<Dependency>) -> Result<()> {
    let value: toml::Value = toml::from_str(text)?;
    let project = value.get("project");
    let name = project.and_then(|v| v.get("name")).and_then(toml::Value::as_str).map(str::to_owned)
        .or_else(|| value.get("tool").and_then(|v| v.get("poetry")).and_then(|v| v.get("name")).and_then(toml::Value::as_str).map(str::to_owned));
    packages.push(Package { repo: repo.into(), manifest: path.into(), ecosystem: "python".into(), name });
    if let Some(items) = project.and_then(|v| v.get("dependencies")).and_then(toml::Value::as_array) {
        for item in items.iter().filter_map(toml::Value::as_str) {
            let package = item.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '=' | '!' | '~' | '[' | ';')).next().unwrap_or(item).to_owned();
            dependencies.push(Dependency { repo: repo.into(), manifest: path.into(), ecosystem: "python".into(), package, requirement: item.into(), scope: "dependencies".into(), local_provider: None });
        }
    }
    if let Some(items) = value.get("tool").and_then(|v| v.get("poetry")).and_then(|v| v.get("dependencies")).and_then(toml::Value::as_table) {
        for (package, requirement) in items {
            if package == "python" { continue; }
            dependencies.push(Dependency { repo: repo.into(), manifest: path.into(), ecosystem: "python".into(), package: package.clone(), requirement: requirement.as_str().map(str::to_owned).unwrap_or_else(|| requirement.to_string()), scope: "poetry.dependencies".into(), local_provider: None });
        }
    }
    Ok(())
}

fn parse_go(repo: &str, path: &str, text: &str, packages: &mut Vec<Package>, dependencies: &mut Vec<Dependency>) -> Result<()> {
    let mut module = None;
    let mut in_require = false;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if let Some(value) = line.strip_prefix("module ") { module = Some(value.trim().to_owned()); continue; }
        if line == "require (" { in_require = true; continue; }
        if in_require && line == ")" { in_require = false; continue; }
        let dep_line = if in_require { Some(line) } else { line.strip_prefix("require ") };
        if let Some(dep_line) = dep_line {
            let mut parts = dep_line.split_whitespace();
            if let (Some(package), Some(requirement)) = (parts.next(), parts.next()) {
                dependencies.push(Dependency { repo: repo.into(), manifest: path.into(), ecosystem: "go".into(), package: package.into(), requirement: requirement.into(), scope: "require".into(), local_provider: None });
            }
        }
    }
    packages.push(Package { repo: repo.into(), manifest: path.into(), ecosystem: "go".into(), name: module });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn npm_local_link_is_detected() {
        let a = tempfile::tempdir().unwrap(); let b = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("package.json"), r#"{"name":"@x/a","dependencies":{"@x/b":"workspace:*"}}"#).unwrap();
        std::fs::write(b.path().join("package.json"), r#"{"name":"@x/b"}"#).unwrap();
        let roots = BTreeMap::from([("a".into(), a.path().to_path_buf()), ("b".into(), b.path().to_path_buf())]);
        let report = scan(&roots).unwrap();
        assert_eq!(report.local_links, 1);
        assert_eq!(report.dependencies[0].local_provider.as_deref(), Some("b"));
    }
    #[test] fn go_require_block_is_parsed() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("go.mod"), "module example/a\nrequire (\n example/b v1.2.3\n)\n").unwrap();
        let report = scan(&BTreeMap::from([("a".into(), root.path().to_path_buf())])).unwrap();
        assert_eq!(report.dependencies[0].package, "example/b");
    }
}
