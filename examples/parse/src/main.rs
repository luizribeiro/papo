//! Print the parsed fields of IRC lines read from standard input.

use std::{
    borrow::Cow,
    error::Error,
    io::{self, BufRead},
};

use papo_proto::{Command, Ctcp, LineRef, strip_formatting};

fn main() -> Result<(), Box<dyn Error>> {
    for line in io::stdin().lock().lines() {
        let line = line?;
        let line = LineRef::parse(&line)?;
        print_line(&line);
    }
    Ok(())
}

fn print_line(line: &LineRef<'_>) {
    let tags: Vec<_> = line
        .tags()
        .map(|(key, value)| (key.to_owned(), value.map(Cow::into_owned)))
        .collect();
    let params: Vec<_> = line.params().collect();
    print!(
        "command={:?} source={:?} tags={tags:?} params={params:?}",
        line.command(),
        line.source()
    );
    if matches!(line.command(), Command::Privmsg | Command::Notice) {
        let text = line.param(1);
        if let Some(ctcp) = text.and_then(Ctcp::parse) {
            print!(" ctcp={} params={:?}", ctcp.command, ctcp.params);
        } else {
            let text = text.map(strip_formatting);
            print!(" text={text:?}");
        }
    }
    println!();
}
