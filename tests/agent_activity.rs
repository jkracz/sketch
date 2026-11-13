//! Agent activity: MCP calls mark the frames each agent works on, `finish_working_on_nodes`
//! clears them, and the view stays where the user left it.

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, Implementation, InitializeRequestParams};
use serde_json::{Value, json};
use sketch_rs::mcp::{self, Caller, Out, exec};
use sketch_rs::model::NodeId;
use sketch_rs::project::Store;
use std::collections::HashSet;

fn host() -> exec::SessionHost {
    let dir = std::env::temp_dir().join(format!("sketch-rs-agents-{:016x}", fastrand::u64(..)));
    let mut h = exec::SessionHost::new(Store::open(dir).unwrap());
    h.session.create(&mut h.ed, "Agents").unwrap();
    h
}

fn caller(agent: u64, client: &str) -> Caller {
    Caller {
        agent,
        client: Some(client.into()),
    }
}

fn call_as(h: &mut exec::SessionHost, c: &Caller, now: f64, name: &str, args: Value) -> Value {
    match exec::call_as(h, Some(c), now, name, &args) {
        Ok(outs) => match outs.into_iter().next() {
            Some(Out::Text(t)) => serde_json::from_str(&t).unwrap_or(Value::String(t)),
            _ => Value::Null,
        },
        Err(e) => panic!("{name} failed: {e}"),
    }
}

fn num(id: &Value) -> NodeId {
    NodeId(
        id.as_str()
            .unwrap()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .unwrap(),
    )
}

fn busy(h: &exec::SessionHost, now: f64) -> HashSet<NodeId> {
    let page = h.ed.active_page_id().to_string();
    h.ed.agents.busy_frames(&page, &h.ed.doc, now)
}

fn artboard(h: &mut exec::SessionHost, c: &Caller, name: &str) -> Value {
    let styles = json!({"width": "320px", "height": "200px"});
    call_as(
        h,
        c,
        0.0,
        "create_artboard",
        json!({"name": name, "styles": styles}),
    )["id"]
        .clone()
}

#[test]
fn writes_and_reads_mark_the_frame_per_agent() {
    let mut h = host();
    let claude = caller(1, "claude-code");
    let cursor = caller(2, "cursor");
    let a = artboard(&mut h, &claude, "A");
    let b = artboard(&mut h, &claude, "B");
    // Editing inside A marks A; a read of B by another agent marks B.
    call_as(
        &mut h,
        &claude,
        10.0,
        "write_html",
        json!({"targetNodeId": a, "mode": "insert-children", "html": "<p>Hi</p>"}),
    );
    call_as(&mut h, &cursor, 20.0, "get_node_info", json!({"nodeId": b}));
    assert_eq!(busy(&h, 30.0), HashSet::from([num(&a), num(&b)]));
    let names: Vec<String> = h.ed.agents.active(30.0).map(|(_, a)| a.label()).collect();
    assert_eq!(names, ["Claude Code", "Cursor"]);

    // Cursor's reads expire on their own; Claude's frames stay while it keeps working.
    let later = 20.0 + sketch_rs::agents::EXPIRE_MS;
    call_as(
        &mut h,
        &claude,
        later - 1.0,
        "get_children",
        json!({"nodeId": a}),
    );
    assert_eq!(busy(&h, later), HashSet::from([num(&a)]));
}

#[test]
fn finish_clears_the_listed_frames_then_everything() {
    let mut h = host();
    let c = caller(1, "codex");
    let a = artboard(&mut h, &c, "A");
    let b = artboard(&mut h, &c, "B");
    let made = call_as(
        &mut h,
        &c,
        1.0,
        "write_html",
        json!({"targetNodeId": a, "mode": "insert-children", "html": "<p>Hi</p>"}),
    );
    let child = made["createdNodeIds"][0].clone();
    call_as(&mut h, &c, 2.0, "get_node_info", json!({"nodeId": child}));
    assert_eq!(busy(&h, 3.0).len(), 2);
    // Finishing A also drops what the agent touched inside it.
    call_as(
        &mut h,
        &c,
        3.0,
        "finish_working_on_nodes",
        json!({"nodeIds": [a]}),
    );
    assert_eq!(busy(&h, 4.0), HashSet::from([num(&b)]));
    call_as(&mut h, &c, 4.0, "finish_working_on_nodes", json!({}));
    assert!(busy(&h, 5.0).is_empty());
    assert_eq!(h.ed.agents.active(5.0).count(), 0);
}

#[test]
fn deleting_a_child_keeps_its_frame_and_a_frame_drops_out() {
    let mut h = host();
    let c = caller(1, "claude-code");
    let a = artboard(&mut h, &c, "A");
    let b = artboard(&mut h, &c, "B");
    let made = call_as(
        &mut h,
        &c,
        1.0,
        "write_html",
        json!({"targetNodeId": a, "mode": "insert-children", "html": "<p>Hi</p>"}),
    );
    call_as(&mut h, &c, 2.0, "finish_working_on_nodes", json!({}));
    let child = made["createdNodeIds"][0].clone();
    call_as(&mut h, &c, 3.0, "delete_nodes", json!({"nodeIds": [child]}));
    assert_eq!(busy(&h, 4.0), HashSet::from([num(&a)]));
    call_as(&mut h, &c, 5.0, "finish_working_on_nodes", json!({}));
    call_as(&mut h, &c, 6.0, "delete_nodes", json!({"nodeIds": [b]}));
    assert!(busy(&h, 7.0).is_empty());
    assert_eq!(h.ed.agents.active(7.0).count(), 0);
}

#[test]
fn opening_another_project_clears_activity() {
    let mut h = host();
    let c = caller(1, "claude-code");
    artboard(&mut h, &c, "A");
    assert_eq!(h.ed.agents.active(1.0).count(), 1);
    h.session.create(&mut h.ed, "Other").unwrap();
    assert_eq!(h.ed.agents.active(1.0).count(), 0);
}

#[test]
fn arrow_endpoints_mark_their_frames() {
    let mut h = host();
    let c = caller(1, "claude-code");
    let a = artboard(&mut h, &c, "A");
    let b = artboard(&mut h, &c, "B");
    call_as(&mut h, &c, 1.0, "finish_working_on_nodes", json!({}));
    let arrow =
        json!({"type": "arrow", "points": [[0, 0], [100, 0]], "startNodeId": a, "endNodeId": b});
    call_as(&mut h, &c, 2.0, "create_shape", arrow);
    let marked = busy(&h, 3.0);
    assert!(
        marked.contains(&num(&a)) && marked.contains(&num(&b)),
        "{marked:?}"
    );
}

#[test]
fn session_end_removes_the_agent() {
    let mut h = host();
    let c = caller(7, "claude-code");
    artboard(&mut h, &c, "A");
    assert_eq!(h.ed.agents.active(1.0).count(), 1);
    exec::call_as(&mut h, Some(&c), 1.0, mcp::SESSION_ENDED, &Value::Null).unwrap();
    assert_eq!(h.ed.agents.active(1.0).count(), 0);
}

#[test]
fn background_page_work_marks_that_page_not_the_view() {
    let mut h = host();
    let c = caller(1, "claude-code");
    artboard(&mut h, &c, "Here");
    let page = call_as(&mut h, &c, 0.0, "create_page", json!({"name": "Other"}));
    let pid = page["pageId"].as_str().unwrap().to_string();
    let styles = json!({"width": "100px", "height": "100px"});
    call_as(
        &mut h,
        &c,
        1.0,
        "create_artboard",
        json!({"name": "There", "styles": styles, "pageId": pid}),
    );
    let pages = h.ed.agents.busy_pages(2.0);
    assert!(pages.contains(pid.as_str()), "{pages:?}");
    assert_eq!(h.ed.page_name(), "Page 1");
}

#[test]
fn agent_edits_leave_the_camera_alone() {
    let mut h = host();
    let c = caller(1, "claude-code");
    let a = artboard(&mut h, &c, "A");
    // The user pans away; the agent keeps editing.
    h.ed.camera.pan.x += 5000.0;
    let camera = h.ed.camera;
    call_as(
        &mut h,
        &c,
        1.0,
        "write_html",
        json!({"targetNodeId": a, "mode": "insert-children", "html": "<p>Hi</p>"}),
    );
    artboard(&mut h, &c, "B");
    assert_eq!(h.ed.camera, camera);
}

#[test]
fn the_frame_mark_fades_in_and_animates() {
    let mut h = host();
    let c = caller(1, "claude-code");
    let a = artboard(&mut h, &c, "A");
    h.ed.tick(0.0);
    let (id, m) = h.ed.agents.marks().next().unwrap();
    assert_eq!(id, num(&a));
    assert!(h.ed.agents.animating());
    assert_eq!(m.alpha(sketch_rs::agents::FADE_MS), 1.0);
    let lo = (0..30)
        .map(|i| sketch_rs::render::scene::agent_pulse(i as f64 * 100.0))
        .fold(1.0, f64::min);
    assert!((0.19..0.3).contains(&lo), "{lo}");
}

/// Over HTTP, each session is its own agent named by its client.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_sessions_are_named_agents() {
    let (tx, rx) = std::sync::mpsc::channel::<mcp::Call>();
    let worker = std::thread::spawn(move || {
        let mut h = host();
        let mut seen = Vec::new();
        while let Ok(c) = rx.recv() {
            let r = exec::call_as(&mut h, c.caller.as_ref(), 0.0, &c.name, &c.args);
            let _ = c.reply.send(r);
            let names: Vec<String> = h.ed.agents.active(1.0).map(|(_, a)| a.label()).collect();
            seen.push(names);
        }
        seen
    });
    let tx = std::sync::Mutex::new(tx);
    let running = mcp::server::spawn(
        0,
        std::sync::Arc::new(move |c| tx.lock().unwrap().send(c).is_ok()),
    )
    .unwrap();
    let info = InitializeRequestParams::new(
        Default::default(),
        Implementation::new("claude-code", "1.0"),
    );
    let transport = rmcp::transport::StreamableHttpClientTransport::from_uri(running.url());
    let client = info.serve(transport).await.unwrap();
    let mut p = CallToolRequestParams::new("create_artboard");
    p.arguments = json!({"name": "A", "styles": {"width": "100px", "height": "100px"}})
        .as_object()
        .cloned();
    client.call_tool(p).await.unwrap();
    client.cancel().await.unwrap();
    running.stop();
    let seen = worker.join().unwrap();
    assert!(
        seen.iter().any(|n| n == &["Claude Code".to_string()]),
        "{seen:?}"
    );
}
