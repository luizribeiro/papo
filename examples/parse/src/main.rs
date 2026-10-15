//! Print the parsed fields of IRC messages read from standard input.

use std::{
    borrow::Cow,
    error::Error,
    io::{self, BufRead},
};

use papo_proto::{Command, Ctcp, MessageRef, strip_formatting};

fn main() -> Result<(), Box<dyn Error>> {
    for line in io::stdin().lock().lines() {
        let line = line?;
        let message = MessageRef::parse(&line)?;
        print_message(&message);
    }
    Ok(())
}

fn print_message(message: &MessageRef<'_>) {
    let tags: Vec<_> = message
        .tags()
        .map(|(key, value)| (key.to_owned(), value.map(Cow::into_owned)))
        .collect();
    let params: Vec<_> = message.params().collect();
    print!(
        "command={:?} source={:?} tags={tags:?} params={params:?}",
        message.command(),
        message.source()
    );
    if matches!(message.command(), Command::Privmsg | Command::Notice) {
        let text = message.param(1);
        if let Some(ctcp) = text.and_then(Ctcp::parse) {
            print!(" ctcp={} params={:?}", ctcp.command, ctcp.params);
        } else {
            let text = text.map(strip_formatting);
            print!(" text={text:?}");
        }
    }
    println!();
}
