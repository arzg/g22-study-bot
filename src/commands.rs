mod assignments;

pub(crate) use assignments::*;

use serenity::prelude::TypeMapKey;
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) struct Config;

impl TypeMapKey for Config {
    type Value = Arc<PathBuf>;
}
