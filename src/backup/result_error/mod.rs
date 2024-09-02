use std::fmt::Debug;
pub mod error;
pub mod result;

pub trait WithDebugObjectAndFnName<S: Into<String>, O: Debug + 'static> {
    fn with_debug_object_and_fn_name(self, obj: O, fn_name: S) -> Self;
}

pub trait WithMsg<S: Into<String>> {
    fn with_msg(self, msg: S) -> Self;
}
