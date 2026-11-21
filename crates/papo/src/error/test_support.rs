use crate::error::{Refusal, ServerError};

impl ServerError {
    /// Creates a numeric refusal using the same classifier as received replies.
    #[doc(hidden)]
    #[must_use]
    pub fn from_numeric<I, S>(code: u16, params: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let params = params.into_iter().map(Into::into).collect::<Vec<_>>();
        if let Some(numeric) = papo_proto::Numeric::new(code) {
            return Self::from_core(papo_core::ServerError::__numeric(numeric, params));
        }
        Self {
            refusal: Refusal::Other,
            numeric: Some(code),
            command: None,
            code: code.to_string().into(),
            description: params.last().cloned().unwrap_or_default(),
            params,
        }
    }

    /// Creates a standard-reply refusal using the same classifier as received replies.
    #[doc(hidden)]
    #[must_use]
    pub fn standard(
        command: impl Into<crate::Command>,
        code: impl Into<crate::ReplyCode>,
        description: impl Into<String>,
    ) -> Self {
        let command: crate::Command = command.into();
        let code: crate::ReplyCode = code.into();
        Self::from_core(papo_core::ServerError::__standard(
            command.as_str().to_owned(),
            code.as_str().to_owned(),
            Vec::new(),
            description.into(),
        ))
    }

    /// Supplies the standard reply's diagnostic parameters in wire order.
    #[doc(hidden)]
    #[must_use]
    pub fn context<I, S>(mut self, context: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.params = context.into_iter().map(Into::into).collect();
        self
    }
}
