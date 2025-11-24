pub mod archive;
mod arcvec;
pub mod backup_config;
pub mod compress;
pub mod encrypt;
pub mod file_ext;
pub mod finish;
mod notifications;
mod redacted;
pub mod result_error;
pub mod retention;
pub mod tar;
pub mod validate;

macro_rules! function_path {
    () => {
        concat!(
            module_path!(),
            "::",
            function_name!(),
            " ",
            file!(),
            ":",
            line!()
        )
    };
}

pub(crate) use function_path;
