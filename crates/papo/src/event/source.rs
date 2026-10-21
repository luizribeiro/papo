use crate::Casemapping;

#[doc(inline)]
pub use papo_core::Source;

pub(crate) fn from_proto(source: papo_proto::Source<'_>, casemapping: Casemapping) -> Source {
    papo_core::Source::from_proto(source, casemapping)
}

pub(crate) fn from_prefix(prefix: String, casemapping: Casemapping) -> Source {
    let message = papo_proto::Line::new("NOTICE", ["*"]).with_source(prefix);
    from_proto(
        message.as_ref().source().expect("owned source is present"),
        casemapping,
    )
}

pub(crate) fn from_name(name: &str, casemapping: Casemapping) -> Source {
    if name.contains('.') {
        from_proto(papo_proto::Source::Server(name), casemapping)
    } else {
        from_proto(papo_proto::Source::__user(name, None, None), casemapping)
    }
}
