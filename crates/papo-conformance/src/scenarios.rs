use super::{Requirements, Scenario, ScenarioFuture, ScenarioSpec};

pub(super) const fn all() -> &'static [ScenarioSpec] {
    &[ScenarioSpec {
        name: "registration",
        doc: "registers with the requested nickname",
        requires: Requirements {
            caps: &[],
            isupport: &[],
        },
        run: registration,
    }]
}

fn registration(context: Scenario) -> ScenarioFuture {
    Box::pin(async move {
        let nick = context.unique();
        let connection = context.client(&nick).await?;
        if connection.welcome.nick != nick {
            return Err("welcome nickname differs from the requested nickname".into());
        }
        connection.close().await
    })
}
