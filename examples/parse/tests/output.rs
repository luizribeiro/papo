//! Output test for the parse example.
#![cfg(not(target_family = "wasm"))]

use std::{
    io::Write,
    process::{Command, Stdio},
};

const TRANSCRIPT: &str = concat!(
    ":irc.example.test CAP * LS * :account-notify batch echo-message message-tags server-time\n",
    ":irc.example.test 001 papo-test :Welcome to ErgoTest, papo-test\n",
    ":irc.example.test 005 papo-test CHANTYPES=#& PREFIX=(qaohv)~&@%+ CHATHISTORY=1000 :are supported by this server\n",
    "@time=2026-10-14T20:15:40.000Z :alice!~alice@users.example.test JOIN #papo * :Alice Example\n",
    ":irc.example.test 353 papo-test = #papo :@Alice +bob papo-test\n",
    ":irc.example.test 366 papo-test #papo :End of NAMES list\n",
    "@time=2026-10-14T20:15:42.123Z;msgid=abcDEF123 :alice!~alice@users.example.test PRIVMSG #papo :hello \x0304,12color\x03 and \x02bold\x02\n",
    "@time=2026-10-14T20:15:43.000Z :alice!~alice@users.example.test PRIVMSG #papo :\x01ACTION waves\x01\n",
    "@+typing=active :alice!~alice@users.example.test TAGMSG #papo\n",
    ":irc.example.test BATCH +history chathistory #papo\n",
    "@batch=history;time=2026-10-14T19:00:00.000Z :bob!~bob@users.example.test PRIVMSG #papo :earlier message\n",
    ":irc.example.test BATCH -history\n",
    ":alice!~alice@users.example.test ACCOUNT alice\n",
    ":alice!~alice@users.example.test CHGHOST ~alice cloak.example\n",
    "PING :irc.example.test\n",
);

const EXPECTED: &str = concat!(
    "command=Cap source=Some(Server(\"irc.example.test\")) tags=[] params=[\"*\", \"LS\", \"*\", \"account-notify batch echo-message message-tags server-time\"]\n",
    "command=Numeric(Numeric(1)) source=Some(Server(\"irc.example.test\")) tags=[] params=[\"papo-test\", \"Welcome to ErgoTest, papo-test\"]\n",
    "command=Numeric(Numeric(5)) source=Some(Server(\"irc.example.test\")) tags=[] params=[\"papo-test\", \"CHANTYPES=#&\", \"PREFIX=(qaohv)~&@%+\", \"CHATHISTORY=1000\", \"are supported by this server\"]\n",
    "command=Join source=Some(User { nick: \"alice\", username: Some(\"~alice\"), host: Some(\"users.example.test\") }) tags=[(\"time\", Some(\"2026-10-14T20:15:40.000Z\"))] params=[\"#papo\", \"*\", \"Alice Example\"]\n",
    "command=Numeric(Numeric(353)) source=Some(Server(\"irc.example.test\")) tags=[] params=[\"papo-test\", \"=\", \"#papo\", \"@Alice +bob papo-test\"]\n",
    "command=Numeric(Numeric(366)) source=Some(Server(\"irc.example.test\")) tags=[] params=[\"papo-test\", \"#papo\", \"End of NAMES list\"]\n",
    "command=Privmsg source=Some(User { nick: \"alice\", username: Some(\"~alice\"), host: Some(\"users.example.test\") }) tags=[(\"time\", Some(\"2026-10-14T20:15:42.123Z\")), (\"msgid\", Some(\"abcDEF123\"))] params=[\"#papo\", \"hello \\u{3}04,12color\\u{3} and \\u{2}bold\\u{2}\"]\n",
    "command=Privmsg source=Some(User { nick: \"alice\", username: Some(\"~alice\"), host: Some(\"users.example.test\") }) tags=[(\"time\", Some(\"2026-10-14T20:15:43.000Z\"))] params=[\"#papo\", \"\\u{1}ACTION waves\\u{1}\"]\n",
    "command=Tagmsg source=Some(User { nick: \"alice\", username: Some(\"~alice\"), host: Some(\"users.example.test\") }) tags=[(\"+typing\", Some(\"active\"))] params=[\"#papo\"]\n",
    "command=Batch source=Some(Server(\"irc.example.test\")) tags=[] params=[\"+history\", \"chathistory\", \"#papo\"]\n",
    "command=Privmsg source=Some(User { nick: \"bob\", username: Some(\"~bob\"), host: Some(\"users.example.test\") }) tags=[(\"batch\", Some(\"history\")), (\"time\", Some(\"2026-10-14T19:00:00.000Z\"))] params=[\"#papo\", \"earlier message\"]\n",
    "command=Batch source=Some(Server(\"irc.example.test\")) tags=[] params=[\"-history\"]\n",
    "command=Account source=Some(User { nick: \"alice\", username: Some(\"~alice\"), host: Some(\"users.example.test\") }) tags=[] params=[\"alice\"]\n",
    "command=Chghost source=Some(User { nick: \"alice\", username: Some(\"~alice\"), host: Some(\"users.example.test\") }) tags=[] params=[\"~alice\", \"cloak.example\"]\n",
    "command=Ping source=None tags=[] params=[\"irc.example.test\"]\n",
);

#[test]
fn prints_a_realistic_ergo_transcript() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_papo-parse"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(TRANSCRIPT.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), EXPECTED);
}
