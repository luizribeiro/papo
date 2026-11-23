use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

use super::{WireIo, client_line, server_line};

fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[tokio::test(flavor = "current_thread")]
#[should_panic(expected = "wire fixture I/O was interrupted")]
async fn cancelling_server_line_closes_and_poisons_the_fixture() {
    let (server, mut client) = duplex(5);
    let mut server = WireIo::new(server);
    let line = b"PING :ergo.test\r\n";
    drop(server_line(&mut server, line));
    let mut write = Box::pin(server_line(&mut server, line));
    assert!(poll(write.as_mut()).is_pending());
    drop(write);

    let mut partial = Vec::new();
    assert!(matches!(
        poll(Box::pin(client.read_to_end(&mut partial)).as_mut()),
        Poll::Ready(Ok(5))
    ));
    assert_eq!(partial, b"PING ");
    server_line(&mut server, line).await;
}

#[tokio::test(flavor = "current_thread")]
#[should_panic(expected = "wire fixture I/O was interrupted")]
async fn cancelling_client_line_closes_and_poisons_the_fixture() {
    let (server, mut client) = duplex(64);
    let mut server = WireIo::new(server);
    client.write_all(b"PONG ").await.unwrap();
    let mut read = Box::pin(client_line(&mut server, b"PONG :ergo.test\r\n"));
    assert!(poll(read.as_mut()).is_pending());
    drop(read);

    assert!(matches!(
        poll(Box::pin(client.read(&mut [0])).as_mut()),
        Poll::Ready(Ok(0))
    ));
    client_line(&mut server, b"PONG :ergo.test\r\n").await;
}
