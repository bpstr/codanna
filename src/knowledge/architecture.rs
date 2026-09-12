//! Deterministic architecture discovery over trusted graph edges.
use crate::knowledge::*;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Serialize)]
pub struct Hub {
    pub id: String,
    pub label: String,
    pub degree: usize,
    pub repos: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Community {
    pub id: usize,
    pub label: String,
    pub members: Vec<String>,
    pub repositories: Vec<String>,
    pub internal_edges: usize,
    pub boundary_edges: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchitectureReport {
    pub nodes: usize,
    pub edges: usize,
    pub connected_components: usize,
    pub communities: Vec<Community>,
    pub hubs: Vec<Hub>,
    pub cross_repository_edges: usize,
    pub unresolved_references: usize,
    pub limitations: Vec<String>,
}

fn usable(edge: &Edge) -> bool { edge.basis != Basis::Candidate }

fn adjacency(graph: &Graph) -> BTreeMap<String, BTreeSet<String>> {
    let mut map = BTreeMap::<String, BTreeSet<String>>::new();
    for id in graph.nodes.keys() { map.entry(id.clone()).or_default(); }
    for edge in graph.edges.iter().filter(|e| usable(e)) {
        map.entry(edge.from.clone()).or_default().insert(edge.to.clone());
        map.entry(edge.to.clone()).or_default().insert(edge.from.clone());
    }
    map
}

fn components(graph: &Graph) -> Vec<Vec<String>> {
    let adj = adjacency(graph);
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for start in graph.nodes.keys() {
        if !seen.insert(start.clone()) { continue; }
        let mut queue = VecDeque::from([start.clone()]);
        let mut members = Vec::new();
        while let Some(id) = queue.pop_front() {
            members.push(id.clone());
            for next in adj.get(&id).into_iter().flatten() {
                if seen.insert(next.clone()) { queue.push_back(next.clone()); }
            }
        }
        members.sort();
        out.push(members);
    }
    out.sort_by(|a,b| b.len().cmp(&a.len()).then_with(|| a.first().cmp(&b.first())));
    out
}

/// Label propagation with deterministic ordering/tie-breaking. This is a structural grouping, not semantic truth.
fn communities(graph: &Graph) -> Vec<Community> {
    let adj = adjacency(graph);
    let mut labels: BTreeMap<String, String> = graph.nodes.keys().map(|id| (id.clone(), id.clone())).collect();
    for _ in 0..20 {
        let mut changed = false;
        for id in graph.nodes.keys() {
            let mut counts = BTreeMap::<String, usize>::new();
            for next in adj.get(id).into_iter().flatten() {
                *counts.entry(labels[next].clone()).or_default() += 1;
            }
            let Some((best, _)) = counts.into_iter().max_by(|(la,ca),(lb,cb)| ca.cmp(cb).then_with(|| lb.cmp(la))) else { continue; };
            if best < labels[id] && best != labels[id] { labels.insert(id.clone(), best); changed = true; }
        }
        if !changed { break; }
    }
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for (id,label) in labels { groups.entry(label).or_default().push(id); }
    let mut result = Vec::new();
    for (index, (_label, mut members)) in groups.into_iter().enumerate() {
        members.sort();
        let set: BTreeSet<_> = members.iter().cloned().collect();
        let mut internal = 0usize; let mut boundary = 0usize;
        let mut repos = BTreeSet::new();
        for id in &members { repos.insert(graph.nodes[id].source.repo.clone()); }
        for edge in graph.edges.iter().filter(|e| usable(e)) {
            let a=set.contains(&edge.from); let b=set.contains(&edge.to);
            if a && b { internal += 1; } else if a || b { boundary += 1; }
        }
        let hub = members.iter().max_by(|a,b| {
            let da=adj.get(*a).map_or(0,|v|v.len()); let db=adj.get(*b).map_or(0,|v|v.len());
            da.cmp(&db).then_with(|| b.cmp(a))
        }).unwrap();
        result.push(Community { id:index, label:graph.nodes[hub].label.clone(), members,
            repositories:repos.into_iter().collect(), internal_edges:internal, boundary_edges:boundary });
    }
    result.sort_by(|a,b| b.members.len().cmp(&a.members.len()).then_with(|| a.label.cmp(&b.label)));
    for (i,c) in result.iter_mut().enumerate() { c.id=i; }
    result
}

pub fn analyze(graph: &Graph, hub_limit: usize) -> Result<ArchitectureReport> {
    graph.validate()?;
    if !(1..=100).contains(&hub_limit) { return Err("hub limit must be 1..100".into()); }
    let adj=adjacency(graph);
    let mut hubs:Vec<_>=graph.nodes.values().map(|node| Hub { id:node.id.clone(), label:node.label.clone(), degree:adj.get(&node.id).map_or(0,|v|v.len()), repos:vec![node.source.repo.clone()] }).collect();
    hubs.sort_by(|a,b| b.degree.cmp(&a.degree).then_with(|| a.id.cmp(&b.id))); hubs.truncate(hub_limit);
    let cross_repository_edges=graph.edges.iter().filter(|e| usable(e) && graph.nodes[&e.from].source.repo != graph.nodes[&e.to].source.repo).count();
    let comps=components(graph);
    Ok(ArchitectureReport { nodes:graph.nodes.len(), edges:graph.edges.iter().filter(|e|usable(e)).count(), connected_components:comps.len(), communities:communities(graph), hubs, cross_repository_edges, unresolved_references:graph.unresolved.len(), limitations:vec![
        "Communities are deterministic structural groups, not domain truth or ownership boundaries.".into(),
        "Hub degree measures graph connectivity, not runtime importance, performance impact, or business criticality.".into(),
        "Candidate relationships are excluded; unresolved references can fragment components and communities.".into(),
    ] })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn deterministic_and_candidate_free() {
        let input=Input{repo:"r".into(),files:BTreeMap::from([("a.rs".into(),"fn a(){}\nfn b(){}".into())]),symbols:vec![
            CodeSymbol{key:1,name:"a".into(),qualified_name:"a".into(),signature:"fn a()".into(),path:"a.rs".into(),start_line:1,end_line:1},
            CodeSymbol{key:2,name:"b".into(),qualified_name:"b".into(),signature:"fn b()".into(),path:"a.rs".into(),start_line:2,end_line:2}],edges:vec![CodeEdge{from:1,to:2,relation:"calls".into()}],..Input::default()};
        let graph=links::build(&input).unwrap(); let a=analyze(&graph,10).unwrap(); let b=analyze(&graph,10).unwrap();
        assert_eq!(serde_json::to_vec(&a).unwrap(),serde_json::to_vec(&b).unwrap()); assert_eq!(a.connected_components,1);
    }
    #[test] fn candidate_does_not_join_components() {
        let mut graph=Graph::default();
        for (i,label) in ["a","b"].into_iter().enumerate(){let path=format!("{label}.md");let hash=digest(label);graph.repositories.entry("r".into()).or_default().files.insert(path.clone(),hash.clone());let id=identity("r","Document",&path,"");graph.nodes.insert(id.clone(),Node{id,kind:Kind::Document,label:label.into(),source:Span{repo:"r".into(),path,start_line:1,end_line:1,hash},excerpt:String::new()});if i==1{}}
        let ids:Vec<_>=graph.nodes.keys().cloned().collect(); graph.edges.push(Edge{from:ids[0].clone(),to:ids[1].clone(),relation:"similar".into(),basis:Basis::Candidate,evidence:graph.nodes[&ids[0]].source.clone(),method:"test".into()});
        graph.validate().unwrap(); assert_eq!(analyze(&graph,2).unwrap().connected_components,2);
    }
}
