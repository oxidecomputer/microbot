// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use http::Extensions;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A shared context struct that is provided to all handlers functions upon invocation. This
/// can be used to store stateful data between executions or share data across commands.
#[derive(Clone)]
pub struct MessengerContext {
    extensions: Arc<RwLock<Extensions>>,
}

impl MessengerContext {
    pub fn new() -> Self {
        Self {
            extensions: Arc::new(RwLock::new(Extensions::new())),
        }
    }

    /// Stores shared data of type `T` into the context. This will overwrite any other data of
    /// the same type (if any is currently stored). If data has been overwritten, the previous
    /// value will be returned.
    pub fn insert<T>(
        &self,
        val: T,
    ) -> Result<Option<Arc<T>>, PoisonError<RwLockWriteGuard<'_, Extensions>>>
    where
        T: Send + Sync + 'static,
    {
        Ok(self.extensions.write()?.insert::<Arc<T>>(Arc::new(val)))
    }

    /// Retrieve shared data of type `T` from the context. Returns `None` if the context does not
    /// contain any data of type `T`
    pub fn get<T>(&self) -> Result<Option<Arc<T>>, PoisonError<RwLockReadGuard<'_, Extensions>>>
    where
        T: Send + Sync + 'static,
    {
        Ok(self.extensions.read()?.get::<Arc<T>>().cloned())
    }
}
