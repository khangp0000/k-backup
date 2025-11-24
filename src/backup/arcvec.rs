//! Thread-safe reference-counted vector wrapper.
//!
//! Provides `ArcVec<T>` for sharing vector data across threads with validation support.

use bon::Builder;
use derive_more::{Deref, DerefMut, From};
use serde::{Deserialize, Serialize};
use validator::{Validate, ValidateLength};

use std::ops::Deref;
use std::sync::Arc;

#[derive(
    From,
    Clone,
    Debug,
    Serialize,
    Deserialize,
    Ord,
    PartialOrd,
    Eq,
    PartialEq,
    Builder,
    Validate,
    Deref,
    DerefMut,
)]
#[serde(transparent)]
pub struct ArcVec<T> {
    inner: Arc<Vec<T>>,
}

impl<T> ArcVec<T> {}

impl<T> Default for ArcVec<T> {
    fn default() -> Self {
        Vec::default().into()
    }
}

impl<T> ValidateLength<usize> for ArcVec<T> {
    fn length(&self) -> Option<usize> {
        Some(self.inner.len())
    }
}

impl<T> From<Vec<T>> for ArcVec<T> {
    fn from(value: Vec<T>) -> Self {
        Self::builder().inner(Arc::new(value)).build()
    }
}

impl<T> AsRef<[T]> for ArcVec<T> {
    fn as_ref(&self) -> &[T] {
        self.inner.deref().as_ref()
    }
}

macro_rules! impl_into_arcvec {
    ($ty:ty) => {
        impl<T> From<$ty> for ArcVec<T>
        where
            $ty: Into<Vec<T>>,
        {
            fn from(val: $ty) -> ArcVec<T> {
                ArcVec::from(val.into())
            }
        }
    };
    (a $ty:ty) => {
        impl<'a, T> From<$ty> for ArcVec<T>
        where
            $ty: Into<Vec<T>>,
        {
            fn from(val: $ty) -> ArcVec<T> {
                ArcVec::from(val.into())
            }
        }
    };
}

impl_into_arcvec!(String);
impl_into_arcvec!(Box<str>);
impl_into_arcvec!(Box<[T]>);
impl_into_arcvec!(a &'a str);
impl_into_arcvec!(a &'a String);
impl_into_arcvec!(a &'a [T]);
