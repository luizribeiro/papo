//! Checks that the copyable browser assets retain their page-to-worker event flow.

#[test]
fn page_and_worker_exchange_configuration_and_events() {
    let page = include_str!("../index.html");
    let worker = include_str!("../worker.js");
    assert!(page.contains("new SharedWorker"));
    assert!(page.contains("worker.port.postMessage"));
    assert!(worker.contains("run(port, data.url, data.nick, data.channel)"));
}
