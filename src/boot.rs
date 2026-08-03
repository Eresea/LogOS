use uefi::{boot, prelude::*, proto::console::gop::GraphicsOutput};

use crate::{
    arch::acpi,
    debug, kernel,
    platform::{entropy, identity, payload, root_key, time},
};

#[entry]
fn main() -> Status {
    debug::write_line(b"LogOS: kernel entered");
    let entropy = entropy::load();
    entropy::announce(entropy);
    let machine = identity::load(entropy.as_ref());
    let secret_root = root_key::load(entropy.as_ref());
    let remote_bootstrap = secret_root.as_ref().and_then(|key| {
        entropy
            .as_ref()
            .and_then(|seed| logos_remote::Bootstrap::from_root(key.bytes(), seed.bytes()).ok())
    });
    root_key::announce(secret_root.as_ref());
    identity::announce(&machine);
    let wall_clock = time::wall_clock();
    time::announce(wall_clock);
    let boot_info = match boot_info() {
        Ok(info) => info,
        Err(_) => return Status::DEVICE_ERROR,
    };
    let acpi = acpi::discover();
    if let Some(tables) = acpi {
        tables.install_reset();
    }
    debug::write_line(b"LogOS: leaving UEFI boot services");

    let payload = payload::stage();
    let memory_map = unsafe { boot::exit_boot_services(None) };
    kernel::main(
        boot_info,
        memory_map,
        acpi,
        machine,
        secret_root,
        remote_bootstrap,
        wall_clock,
        payload,
    )
}

pub(crate) struct Info {
    pub framebuffer: *mut u8,
    pub framebuffer_size: usize,
    pub resolution: (usize, usize),
    pub stride: usize,
}

fn boot_info() -> uefi::Result<Info> {
    let graphics_handle = boot::get_handle_for_protocol::<GraphicsOutput>()?;
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(graphics_handle)?;
    let mode = gop.current_mode_info();
    let (width, height) = mode.resolution();
    let mut framebuffer = gop.frame_buffer();
    Ok(Info {
        framebuffer: framebuffer.as_mut_ptr(),
        framebuffer_size: framebuffer.size(),
        resolution: (width, height),
        stride: mode.stride(),
    })
}
