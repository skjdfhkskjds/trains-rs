#![no_main]
#![no_std]

use core::panic::PanicInfo;

use trains_kernel::runtime::{Command, Runtime};

mod demo;

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(_dtb: usize) -> ! {
    let console = trains_kernel::initialize();
    let mut runtime = Runtime::<_, 1>::new(console);
    runtime
        .register(Command::new("demo", demo::cooperative_yield::run))
        .expect("the demo command registry is valid");
    runtime.run()
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    trains_kernel::park()
}
