//! Interactive application command registration and dispatch.

use core::fmt::Write as _;

use trains_platform::{Console as _, Platform};

use crate::{Kernel, KernelConsole};

const INPUT_CAPACITY: usize = 128;

pub type CommandHandler<P> = fn(kernel: &Kernel<P>, arguments: &str) -> CommandResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandResult {
    Success,
    Failure,
}

#[derive(Clone, Copy)]
pub struct Command<P: Platform> {
    name: &'static str,
    handler: CommandHandler<P>,
}

impl<P: Platform> Command<P> {
    pub const fn new(name: &'static str, handler: CommandHandler<P>) -> Self {
        Self { name, handler }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterError {
    CapacityReached,
    DuplicateName,
    InvalidName,
}

pub struct Runtime<'kernel, P: Platform, const COMMAND_CAPACITY: usize> {
    kernel: &'kernel Kernel<P>,
    console: KernelConsole<P>,
    commands: [Option<Command<P>>; COMMAND_CAPACITY],
}

impl<'kernel, P: Platform, const COMMAND_CAPACITY: usize> Runtime<'kernel, P, COMMAND_CAPACITY> {
    pub fn new(kernel: &'kernel Kernel<P>) -> Self {
        Self {
            console: kernel.console(),
            kernel,
            commands: [None; COMMAND_CAPACITY],
        }
    }

    pub fn register(&mut self, command: Command<P>) -> Result<(), RegisterError> {
        if command.name.is_empty() || command.name.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(RegisterError::InvalidName);
        }
        if self
            .commands
            .iter()
            .flatten()
            .any(|registered| registered.name == command.name)
        {
            return Err(RegisterError::DuplicateName);
        }

        let slot = self
            .commands
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RegisterError::CapacityReached)?;
        *slot = Some(command);
        Ok(())
    }

    pub fn run(mut self) -> ! {
        let mut input = [0; INPUT_CAPACITY];
        let mut discard_line_feed = false;

        loop {
            write!(self.console, "> ").ok();
            let length = self.read_line(&mut input, &mut discard_line_feed);
            let line = core::str::from_utf8(&input[..length])
                .expect("the console accepts ASCII input only");
            self.dispatch(line);
        }
    }

    fn read_line(&mut self, input: &mut [u8], discard_line_feed: &mut bool) -> usize {
        let mut length = 0;

        loop {
            let byte = match self.console.read_byte() {
                Ok(byte) => byte,
                Err(_) => continue,
            };

            if *discard_line_feed {
                *discard_line_feed = false;
                if byte == b'\n' {
                    continue;
                }
            }

            match byte {
                b'\r' => {
                    *discard_line_feed = true;
                    writeln!(self.console).ok();
                    return length;
                }
                b'\n' => {
                    writeln!(self.console).ok();
                    return length;
                }
                8 | 127 if length > 0 => {
                    length -= 1;
                    write!(self.console, "\x08 \x08").ok();
                }
                b' '..=b'~' if length < input.len() => {
                    input[length] = byte;
                    length += 1;
                    write!(self.console, "{}", byte as char).ok();
                }
                b' '..=b'~' => {
                    write!(self.console, "\x07").ok();
                }
                _ => {}
            }
        }
    }

    fn dispatch(&mut self, line: &str) {
        let line = line.trim_ascii();
        if line.is_empty() {
            return;
        }
        let name_end = line
            .bytes()
            .position(|byte| byte.is_ascii_whitespace())
            .unwrap_or(line.len());
        let name = &line[..name_end];
        let arguments = line[name_end..].trim_ascii_start();

        let Some(command) = self
            .commands
            .iter()
            .flatten()
            .find(|command| command.name == name)
            .copied()
        else {
            self.kernel
                .logger()
                .warning(format_args!("unknown command: {name}"))
                .ok();
            return;
        };

        if (command.handler)(self.kernel, arguments) == CommandResult::Failure {
            self.kernel
                .logger()
                .error(format_args!("command failed: {name}"))
                .ok();
        }
    }
}
