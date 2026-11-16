use super::*;

#[tokio::test(flavor = "current_thread")]
async fn wrapped_connections_carry_the_tls_session() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    within(async {
        let server = from_env();
        let nick = unique("papo-wrapped");
        let opened = Arc::new(AtomicUsize::new(0));
        let (client, mut events) = papo::__testing::wrapped::connect_wrapped(
            Client::builder(
                Endpoint::tls("localhost", server.tls_addr().port()).unwrap(),
                support::nick(&nick),
            )
            .tls_ca_pem(std::fs::read(server.ca_pem_path()).unwrap()),
            {
                let opened = Arc::clone(&opened);
                move |stream| {
                    opened.fetch_add(1, Ordering::Relaxed);
                    stream
                }
            },
        )
        .await
        .unwrap();

        wait_connected(&mut events, &nick).await;
        assert_eq!(opened.load(Ordering::Relaxed), 1);
        client.quit().await.unwrap();
    })
    .await;
}
