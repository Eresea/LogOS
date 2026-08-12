pub fn idle() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
