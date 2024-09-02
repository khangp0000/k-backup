use age::stream::StreamWriter;
use liblzma::write::XzEncoder;
use std::io::{Error, Write};

impl<W: Write> Finish<W> for StreamWriter<W> {
    fn finish(self) -> Result<W, Error> {
        self.finish()
    }
}

pub trait Finish<O> {
    fn finish(self) -> Result<O, Error>;
}

impl<W: Write> Finish<W> for XzEncoder<W> {
    fn finish(self) -> Result<W, Error> {
        self.finish()
    }
}
