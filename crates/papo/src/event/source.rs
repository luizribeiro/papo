use crate::Casemapping;

#[doc(inline)]
pub use papo_core::Source;

pub(crate) fn from_proto(source: papo_proto::Source<'_>, casemapping: Casemapping) -> Source {
    papo_core::Source::from_proto(source, casemapping)
}

pub(crate) fn from_proto_with_origin(
    source: papo_proto::Source<'_>,
    casemapping: Casemapping,
    origin: crate::Origin,
) -> Source {
    papo_core::Source::__from_proto(source, casemapping, origin)
}

pub(crate) fn from_prefix(
    prefix: String,
    casemapping: Casemapping,
    origin: crate::Origin,
) -> Source {
    let message = papo_proto::Line::new("NOTICE")
        .with_params(["*"])
        .with_source(prefix);
    from_proto_with_origin(
        message.as_ref().source().expect("owned source is present"),
        casemapping,
        origin,
    )
}

pub(crate) fn from_name(name: &str, casemapping: Casemapping, origin: crate::Origin) -> Source {
    if name.contains('.') {
        from_proto_with_origin(papo_proto::Source::Server(name), casemapping, origin)
    } else {
        from_proto_with_origin(
            papo_proto::Source::__user(name, None, None),
            casemapping,
            origin,
        )
    }
}
