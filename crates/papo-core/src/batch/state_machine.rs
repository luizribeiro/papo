use std::time::Duration;

use papo_proto::LineRef;
use proptest::{prelude::*, prop_oneof};
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest};
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

impl ReferenceStateMachine for BatchModel {
    type State = u8;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self::State> {
        Just(0).boxed()
    }

    fn transitions(depth: &Self::State) -> BoxedStrategy<Self::Transition> {
        match depth {
            0 => prop_oneof![Just(Transition::OpenRoot), Just(Transition::Reset)].boxed(),
            1 => prop_oneof![
                Just(Transition::OpenMetadata),
                Just(Transition::OpenHistory),
                Just(Transition::Member),
                Just(Transition::CloseRoot),
                Just(Transition::Timeout),
                Just(Transition::Reset),
            ]
            .boxed(),
            _ => prop_oneof![
                Just(Transition::Member),
                Just(Transition::CloseChild),
                Just(Transition::Timeout),
                Just(Transition::Reset),
            ]
            .boxed(),
        }
    }

    fn apply(mut depth: Self::State, transition: &Self::Transition) -> Self::State {
        match transition {
            Transition::OpenRoot | Transition::CloseChild => depth = 1,
            Transition::OpenMetadata | Transition::OpenHistory => depth = 2,
            Transition::CloseRoot | Transition::Timeout | Transition::Reset => depth = 0,
            Transition::Member => {}
        }
        depth
    }

    fn preconditions(depth: &Self::State, transition: &Self::Transition) -> bool {
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

struct BatchAccounting;

impl StateMachineTest for BatchAccounting {
    type SystemUnderTest = Batches;
    type Reference = BatchModel;

    fn init_test(_: &u8) -> Self::SystemUnderTest {
        Batches::new(
            &Config::default()
                .max_batch_lines(6)
                .batch_timeout(Duration::from_secs(1)),
        )
    }

    fn apply(mut batches: Batches, depth: &u8, transition: Transition) -> Batches {
        let now = Instant::now();
        let line = match transition {
            Transition::OpenRoot => Some(":ergo.test BATCH +outer vendor/sync"),
            Transition::OpenMetadata => Some("@batch=outer :ergo.test BATCH +inner metadata"),
            Transition::OpenHistory => {
                Some("@batch=outer :ergo.test BATCH +inner chathistory #papo")
            }
            Transition::Member if *depth == 2 => Some(
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

    fn check_invariants(batches: &Batches, _: &u8) {
        assert!(batches.buffered_bytes >= batches.recount());
    }
}

proptest_state_machine::prop_state_machine! {
    #[test]
    fn batch_accounting_covers_every_retained_event(sequential 1..80 => BatchAccounting);
}
