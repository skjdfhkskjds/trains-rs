#![no_main]
#![no_std]

use core::panic::PanicInfo;

use trains_kernel::{ExceptionFrame, Kernel, runtime::Runtime};
use trains_platform::{RASPI4, Raspi4};

mod demo;

pub(crate) static KERNEL: Kernel<Raspi4> = Kernel::new(RASPI4);

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(_dtb: usize) -> ! {
    KERNEL.initialize();
    let mut runtime = Runtime::<Raspi4, 1>::new(&KERNEL);
    runtime
        .register(demo::CooperativeYield::command())
        .expect("the demo command registry is valid");
    runtime.run()
}

#[unsafe(no_mangle)]
extern "C" fn exception_handler(frame: &mut ExceptionFrame) {
    KERNEL.handle_exception(frame);
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    KERNEL.park()
}
