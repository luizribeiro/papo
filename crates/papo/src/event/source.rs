use crate::Casemapping;

#[doc(inline)]
pub use papo_core::Source;

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
