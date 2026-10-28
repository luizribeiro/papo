use std::time::Duration;

use papo_proto::LineRef;
use proptest::prelude::*;
use web_time::Instant;

use super::Batches;
use crate::Config;

#[derive(Clone, Debug)]
enum Transition {
    OpenRoot,
    OpenMetadata,
    OpenHistory,
    Member,
    CloseChild,
    CloseRoot,
    Timeout,
    Reset,
}

struct BatchModel;

impl BatchModel {
    fn transitions(depth: u8) -> &'static [Transition] {
        match depth {
            0 => &[Transition::OpenRoot, Transition::Reset],
            1 => &[
                Transition::OpenMetadata,
                Transition::OpenHistory,
                Transition::Member,
                Transition::CloseRoot,
                Transition::Timeout,
                Transition::Reset,
            ],
            _ => &[
                Transition::Member,
                Transition::CloseChild,
                Transition::Timeout,
                Transition::Reset,
            ],
        }
    }

    fn apply(mut depth: u8, transition: &Transition) -> u8 {
        match transition {
            Transition::OpenRoot | Transition::CloseChild => depth = 1,
            Transition::OpenMetadata | Transition::OpenHistory => depth = 2,
            Transition::CloseRoot | Transition::Timeout | Transition::Reset => depth = 0,
            Transition::Member => {}
        }
        depth
    }

    fn preconditions(depth: u8, transition: &Transition) -> bool {
        match depth {
            0 => matches!(transition, Transition::OpenRoot | Transition::Reset),
            1 => !matches!(transition, Transition::CloseChild),
            _ => matches!(
                transition,
                Transition::Member
                    | Transition::CloseChild
                    | Transition::Timeout
                    | Transition::Reset
            ),
        }
    }
}

fn transitions() -> impl Strategy<Value = Vec<Transition>> {
    proptest::collection::vec(any::<u8>(), 1..80).prop_map(|choices| {
        let mut depth = 0;
        choices
            .into_iter()
            .map(|choice| {
                let available = BatchModel::transitions(depth);
                let transition = available[usize::from(choice) % available.len()].clone();
                depth = BatchModel::apply(depth, &transition);
                transition
            })
            .collect()
    })
}

struct BatchAccounting;

impl BatchAccounting {
    fn init_test() -> Batches {
        Batches::new(
            &Config::default()
                .max_batch_lines(6)
                .batch_timeout(Duration::from_secs(1)),
        )
    }

    fn apply(mut batches: Batches, depth: u8, transition: &Transition) -> Batches {
        let now = Instant::now();
        let line = match transition {
            Transition::OpenRoot => Some(":ergo.test BATCH +outer vendor/sync"),
            Transition::OpenMetadata => Some("@batch=outer :ergo.test BATCH +inner metadata"),
            Transition::OpenHistory => {
                Some("@batch=outer :ergo.test BATCH +inner chathistory #papo")
            }
            Transition::Member if depth == 2 => Some(
                "@batch=inner;time=2026-10-15T12:00:01.000Z;msgid=RKi9Cw \
                 :alice!~alice@users.example PRIVMSG #papo :review is ready",
            ),
            Transition::Member => {
                Some("@batch=outer :alice!~alice@users.example NOTICE papo :maintenance soon")
            }
            Transition::CloseChild => Some("@batch=outer :ergo.test BATCH -inner"),
            Transition::CloseRoot => Some(":ergo.test BATCH -outer"),
            Transition::Timeout => {
                _ = batches.handle_timeout(crate::deadline(now, Duration::from_secs(1)).unwrap());
                None
            }
            Transition::Reset => {
                batches.reset();
                None
            }
        };
        if let Some(line) = line {
            _ = batches.handle(
                now,
                &LineRef::parse(line).unwrap(),
                usize::MAX,
                false,
                false,
            );
        }
        batches
    }

    fn check_invariants(batches: &Batches) -> bool {
        batches.accounting_is_sound()
    }
}

proptest! {
    #[test]
    fn batch_accounting_covers_every_retained_event(transitions in transitions()) {
        let mut depth = 0;
        let mut batches = BatchAccounting::init_test();

        for transition in transitions {
            prop_assert!(BatchModel::preconditions(depth, &transition));
            let next_depth = BatchModel::apply(depth, &transition);
            batches = BatchAccounting::apply(batches, depth, &transition);
            depth = next_depth;
            prop_assert!(BatchAccounting::check_invariants(&batches));
        }
    }
}
