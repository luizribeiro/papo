use crate::driver::test_support::*;
use std::future::IntoFuture;

const CAPS: &str = "draft/channel-rename labeled-response";
const ERGO_ISUPPORT: &[u8] = b":ergo.test 005 papo-driver CHANMODES=Ibe,k,fl,CEMRUimnstu KICKLEN=390 PREFIX=(qaohv)~&@%+ :are supported by this server\r\n";

#[derive(Clone, Copy, Debug)]
enum Request {
    Part,
    Kick,
    Kickban,
    Rename,
    Quit,
}

macro_rules! with_reason {
    ($request:expr, $reason:expr) => {{
        let request = $request;
        let request = match $reason {
            Some(reason) => request.reason(reason),
            None => request,
        };
        let future = request.into_future();
        Box::pin(async move { future.await.map(drop) })
    }};
}

type Pending = std::pin::Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;

fn start(client: &crate::Client, request: Request, reason: Option<&'static str>) -> Pending {
    match request {
        Request::Part => with_reason!(client.part("#papo"), reason),
        Request::Kick => with_reason!(client.kick("#papo", "bob"), reason),
        Request::Kickban => with_reason!(client.kickban("#papo").nick("bob").ban("bob"), reason),
        Request::Rename => with_reason!(client.rename_channel("#papo").to("#papo-dev"), reason),
        Request::Quit => with_reason!(client.quit(), reason),
    }
}

/// Each exchange is a line the client must write and Ergo's answer to it.
fn exchanges(request: Request) -> &'static [(&'static str, &'static str)] {
    match request {
        Request::Part => &[(
            "@label=0 PART #papo",
            "@msgid=x95ke2uf27w64gbhhcnccekj8s;time=2026-11-15T20:07:35.242Z;label=0 :papo-driver!~u@3t4fgnf9zitka.irc PART #papo",
        )],
        Request::Kick => &[(
            "@label=0 KICK #papo bob",
            "@time=2026-11-15T20:07:22.160Z;label=0;msgid=k8gq9fd2udzi77ghui3v88y482 :papo-driver!~u@3t4fgnf9zitka.irc KICK #papo bob papo-driver",
        )],
        Request::Kickban => &[
            (
                "@label=0 MODE #papo +b bob",
                "@time=2026-11-15T20:07:20.004Z;label=0;msgid=rb8w2q5hx4unmhcbg6fsw4qkfe :papo-driver!~u@3t4fgnf9zitka.irc MODE #papo +b bob!*@*",
            ),
            (
                "@label=1 KICK #papo bob",
                "@time=2026-11-15T20:07:22.160Z;label=1;msgid=k8gq9fd2udzi77ghui3v88y482 :papo-driver!~u@3t4fgnf9zitka.irc KICK #papo bob papo-driver",
            ),
        ],
        Request::Rename => &[(
            "@label=0 RENAME #papo #papo-dev",
            "@time=2026-11-15T20:11:30.693Z;label=0 :papo-driver!~u@t86kzv775fsaw.irc RENAME #papo #papo-dev :",
        )],
        Request::Quit => &[("QUIT", "ERROR :Quit: papo-driver")],
    }
}

#[tokio::test(flavor = "current_thread")]
async fn an_empty_reason_sends_the_same_line_as_no_reason() {
    let requests = [
        Request::Part,
        Request::Kick,
        Request::Kickban,
        Request::Rename,
        Request::Quit,
    ];
    for request in requests {
        for reason in [None, Some("")] {
            let (mut server, client, mut events, driver) =
                scripted(config().wanted_caps(CAPS.split(' ')));
            let task = tokio::spawn(driver);
            complete_connect_caps(&mut server, &mut events, CAPS, CAPS).await;
            server_line(&mut server, ERGO_ISUPPORT).await;
            let pending = tokio::spawn(start(&client, request, reason));
            for (sent, reply) in exchanges(request) {
                client_line(&mut server, format!("{sent}\r\n").as_bytes()).await;
                server_line(&mut server, format!("{reply}\r\n").as_bytes()).await;
            }
            if matches!(request, Request::Quit) {
                within(server.shutdown()).await.unwrap();
            }
            let result = within(pending).await.unwrap();
            assert!(result.is_ok(), "{request:?} with {reason:?}: {result:?}");
            task.abort();
        }
    }
}
