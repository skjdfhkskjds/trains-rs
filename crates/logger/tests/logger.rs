use std::convert::Infallible;
use std::fmt;
use std::sync::Mutex;

use trains_logger::{Level, Logger};
use trains_platform::Console;
use trains_platform::io::{ErrorType, Read, ReadReady, Write, WriteReady};

#[derive(Clone, Copy)]
struct Capture(&'static Mutex<String>);

impl Capture {
    fn new() -> Self {
        Self(Box::leak(Box::new(Mutex::new(String::new()))))
    }

    fn output(self) -> String {
        self.0.lock().unwrap().clone()
    }
}

impl ErrorType for Capture {
    type Error = Infallible;
}

impl Read for Capture {
    fn read(&mut self, _buffer: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(0)
    }
}

impl Write for Capture {
    fn write(&mut self, buffer: &[u8]) -> Result<usize, Self::Error> {
        let value = String::from_utf8_lossy(buffer);
        self.0.lock().unwrap().push_str(&value);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ReadReady for Capture {
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
}

impl WriteReady for Capture {
    fn write_ready(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

impl fmt::Write for Capture {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.0.lock().unwrap().push_str(value);
        Ok(())
    }
}

impl Console for Capture {}

#[test]
fn convenience_methods_write_colored_headers_and_one_line_each() {
    let capture = Capture::new();
    let mut logger = Logger::new(capture);

    logger.info("ready").unwrap();
    logger.warning("running warm").unwrap();
    logger.error("failed").unwrap();
    logger.debug("details").unwrap();

    assert_eq!(
        capture.output(),
        concat!(
            "\x1b[94m[INFO]\x1b[0m ready\n",
            "\x1b[33m[WARN]\x1b[0m running warm\n",
            "\x1b[31m[ERROR]\x1b[0m failed\n",
            "\x1b[35m[DEBUG]\x1b[0m details\n",
        )
    );
}

#[test]
fn log_preserves_formatted_arguments() {
    let capture = Capture::new();
    let mut logger = Logger::new(capture);

    logger
        .log(Level::Info, format_args!("task {} ({:#x})", 7, 42))
        .unwrap();

    assert_eq!(capture.output(), "\x1b[94m[INFO]\x1b[0m task 7 (0x2a)\n");
}

#[test]
fn into_inner_returns_the_wrapped_console() {
    let capture = Capture::new();
    let logger = Logger::new(capture);

    let returned = logger.into_inner();

    assert!(std::ptr::eq(capture.0, returned.0));
}
