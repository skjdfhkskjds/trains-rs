pub trait ErrorType {
    type Error;
}

pub trait Read: ErrorType {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error>;
}

pub trait Write: ErrorType {
    fn write(&mut self, buffer: &[u8]) -> Result<usize, Self::Error>;
    fn flush(&mut self) -> Result<(), Self::Error>;
}

pub trait ReadReady: ErrorType {
    fn read_ready(&mut self) -> Result<bool, Self::Error>;
}

pub trait WriteReady: ErrorType {
    fn write_ready(&mut self) -> Result<bool, Self::Error>;
}
