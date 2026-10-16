use std::collections::HashMap;

use papo_core::{MetadataCommand, MetadataValue};

use crate::{Client, Error, driver::CoreRequest};

/// Handle for metadata requests and cached metadata state.
pub struct Metadata<'a> {
    client: &'a Client,
}

impl Client {
    /// Returns a handle for metadata requests and cached state.
    #[must_use]
    pub const fn metadata(&self) -> Metadata<'_> {
        Metadata { client: self }
    }
}

impl Metadata<'_> {
    async fn request(&self, command: MetadataCommand) -> Result<(), Error> {
        let response = self
            .client
            .core_request(CoreRequest::Metadata(command))
            .await?;
        if let Some(error) = response.metadata_error() {
            return Err(Error::Metadata(error));
        }
        response
            .error()
            .map_or(Ok(()), |error| Err(Error::Server(error)))
    }

    /// Gets selected keys and returns the target's resulting cache.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn get<I, S>(
        &self,
        target: &str,
        keys: I,
    ) -> Result<HashMap<String, MetadataValue>, Error>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.request(MetadataCommand::Get {
            target: target.into(),
            keys: keys.into_iter().map(Into::into).collect(),
        })
        .await?;
        Ok(self
            .client
            .state(|state| state.metadata(target).cloned().unwrap_or_default()))
    }

    /// Lists visible keys and returns the target's resulting cache.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn list(&self, target: &str) -> Result<HashMap<String, MetadataValue>, Error> {
        self.request(MetadataCommand::List {
            target: target.into(),
        })
        .await?;
        Ok(self
            .client
            .state(|state| state.metadata(target).cloned().unwrap_or_default()))
    }

    /// Sets or removes one metadata key for the current nickname.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn set(&self, key: &str, value: Option<&str>) -> Result<(), Error> {
        let target = self.client.nick();
        self.set_on(&target, key, value).await
    }

    /// Sets or removes one metadata key on `target`.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn set_on(&self, target: &str, key: &str, value: Option<&str>) -> Result<(), Error> {
        self.request(MetadataCommand::Set {
            target: target.into(),
            key: key.into(),
            value: value.map(str::to_owned),
        })
        .await
    }

    /// Clears all metadata for the current nickname.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn clear(&self) -> Result<(), Error> {
        self.request(MetadataCommand::Clear {
            target: self.client.nick(),
        })
        .await
    }
}
