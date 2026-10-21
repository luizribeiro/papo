//! Print the parsed fields of IRC lines read from standard input.

use std::{error::Error, io};

use papo_proto::LineRef;

fn main() -> Result<(), Box<dyn Error>> {
    for input in io::stdin().lines() {
        let input = input?;
        let line = LineRef::parse(&input)?;
        println!(
            "command={:?} source={:?} tags={:?} params={:?}",
            line.command(),
            line.source(),
            line.tags().collect::<Vec<_>>(),
            line.params().collect::<Vec<_>>()
        );
    }
    Ok(())
}
