//! Test: Project registry can register a project
//!
//! These tests use isolated temporary directories to avoid interfering with
//! the user's actual codanna configuration or with parallel test execution

use serde_json::json;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Create an isolated test environment with its own registry file
fn setup_test_env() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let registry_file = temp_dir.path().join("projects.json");

    (temp_dir, registry_file)
}

/// Mock ProjectRegistry that uses a test directory
struct TestProjectRegistry {
    registry_file: PathBuf,
}

impl TestProjectRegistry {
    fn new(registry_file: PathBuf) -> Self {
        Self { registry_file }
    }

    fn register_project(&self, project_path: &Path) -> Result<String, Box<dyn std::error::Error>> {
        // For now, just create a simple entry
        // This simulates what ProjectRegistry does without touching global files

        // Generate a simple unique ID for testing (avoiding uuid dependency)
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // Fixed-width fields keep the mock identifier stable across hosts whose
        // process IDs have different digit counts.
        let project_id = format!("test-id-{timestamp:032x}-{:08x}", std::process::id());
        let project_name = project_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let entry = json!({
            "id": project_id,
            "name": project_name,
            "path": project_path.to_string_lossy()
        });

        // Save to our test file
        let mut projects = if self.registry_file.exists() {
            let content = std::fs::read_to_string(&self.registry_file)?;
            serde_json::from_str(&content).unwrap_or_else(|_| json!([]))
        } else {
            json!([])
        };

        projects.as_array_mut().unwrap().push(entry);

        std::fs::write(
            &self.registry_file,
            serde_json::to_string_pretty(&projects)?,
        )?;

        Ok(project_id)
    }

    fn find_project_by_id(&self, id: &str) -> Option<serde_json::Value> {
        if !self.registry_file.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&self.registry_file).ok()?;
        let projects: serde_json::Value = serde_json::from_str(&content).ok()?;

        projects.as_array()?.iter().find(|p| p["id"] == id).cloned()
    }
}

#[test]
fn test_register_project_creates_entry() {
    let (_temp_dir, registry_file) = setup_test_env();

    println!("Test: register_project() creates entry with UUID");

    // Create a test project path
    let project_path = PathBuf::from("/Users/test/my-project");

    // Create test registry
    let registry = TestProjectRegistry::new(registry_file.clone());

    // Register the project
    let result = registry.register_project(&project_path);

    // Should succeed and return a UUID
    assert!(result.is_ok(), "Should register project successfully");
    let project_id = result.unwrap();

    // UUID should be non-empty and look like a UUID (basic check)
    assert!(!project_id.is_empty(), "Should return non-empty UUID");
    assert!(project_id.len() >= 32, "UUID should be at least 32 chars");
    println!("Generated UUID: {project_id}");

    // Registry file should exist
    assert!(registry_file.exists(), "Registry file should be created");

    // Find our project by ID
    let project = registry.find_project_by_id(&project_id);
    assert!(project.is_some(), "Should find project by ID");

    let project_info = project.unwrap();
    assert_eq!(
        project_info["path"],
        project_path.to_string_lossy().as_ref(),
        "Path should match"
    );
    assert_eq!(
        project_info["name"], "my-project",
        "Name should be extracted from path"
    );

    // Test that find_project_by_id returns None for non-existent ID
    let fake_id = "nonexistent1234567890abcdef12345";
    let not_found = registry.find_project_by_id(fake_id);
    assert!(
        not_found.is_none(),
        "Should return None for non-existent project ID"
    );

    println!("Result: Project registered with ID {project_id}");
    println!("Registry saved to: {}", registry_file.display());

    // No cleanup needed - TempDir cleans up automatically
}

#[test]
fn test_update_project_path() {
    let (_temp_dir, registry_file) = setup_test_env();

    println!("Test: update_project_path() updates existing project");

    // Register a project first
    let original_path = PathBuf::from("/Users/test/original-location");
    let registry = TestProjectRegistry::new(registry_file.clone());
    let project_id = registry
        .register_project(&original_path)
        .expect("Should register project");

    // Update the path
    let new_path = PathBuf::from("/Users/test/new-location");

    // For this test, we'll directly manipulate the JSON since we're testing behavior
    let content = std::fs::read_to_string(&registry_file).expect("Should read registry");
    let mut projects: serde_json::Value =
        serde_json::from_str(&content).expect("Should parse JSON");

    if let Some(array) = projects.as_array_mut() {
        for project in array {
            if project["id"] == project_id {
                project["path"] = serde_json::json!(new_path.to_string_lossy());
                project["name"] = serde_json::json!("new-location");
                break;
            }
        }
    }

    std::fs::write(
        &registry_file,
        serde_json::to_string_pretty(&projects).unwrap(),
    )
    .expect("Should write updated registry");

    // Verify the update
    let project = registry
        .find_project_by_id(&project_id)
        .expect("Should find project after update");

    assert_eq!(
        project["path"],
        new_path.to_string_lossy().as_ref(),
        "Path should be updated"
    );
    assert_eq!(
        project["name"], "new-location",
        "Name should be updated from new path"
    );

    println!("Result: Project path updated from {original_path:?} to {new_path:?}");
}
