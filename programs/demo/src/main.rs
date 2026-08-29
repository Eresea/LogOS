#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

#[cfg(target_os = "none")]
use logos_abi::{AtriumApp, SurfaceHandle};
#[cfg(target_os = "none")]
use logos_program::{ProgramClient, SurfaceEvent};
#[cfg(target_os = "none")]
use logos_ui::{UiBlueprint, UiComponentTree, UiNodeKind, UiRect, UiText};

#[cfg(target_os = "none")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut client = match unsafe { ProgramClient::from_fixed_bootstrap() } {
        Ok(client) => client,
        Err(_) => idle(),
    };
    let _ = client.request_surface(AtriumApp::Calculator);
    loop {
        let _ = client.retry_surface_request();
        if let Ok(Some(SurfaceEvent::Created(surface))) = client.poll_surface() {
            if let Some(scene) = demo_scene(surface) {
                let _ = client.send_scene(scene.as_slice());
            }
        }
        let mut input = logos_abi::AtriumSurfaceInput::new(
            SurfaceHandle::EMPTY,
            logos_abi::InputMessage::key(
                logos_abi::KeyCode::ESCAPE,
                logos_abi::KeyState::Pressed,
                0,
            ),
        );
        let _ = client.receive_input(&mut input);
        yield_now();
    }
}

#[cfg(target_os = "none")]
fn demo_scene(surface: SurfaceHandle) -> Option<logos_ui_graphics::UiSceneFrame> {
    let mut blueprint = UiBlueprint::new();
    let root = blueprint.push_root(UiNodeKind::Root, 1).ok()?;
    let panel = blueprint.push_child(UiNodeKind::Panel, root, 2).ok()?;
    let label = blueprint.push_child(UiNodeKind::Label, panel, 3).ok()?;
    blueprint.set_text(label, UiText::from_bytes(b"Atrium program")?).ok()?;
    let mut tree = UiComponentTree::from_blueprint(&blueprint).ok()?;
    let viewport = UiRect::new(0, 0, 320, 220);
    let root_handle = tree.tree().handle_at(usize::from(root)).ok()?;
    let panel_handle = tree.tree().handle_at(usize::from(panel)).ok()?;
    let label_handle = tree.tree().handle_at(usize::from(label)).ok()?;
    tree.tree_mut().set_bounds(root_handle, viewport).ok()?;
    tree.tree_mut().set_bounds(panel_handle, UiRect::new(16, 16, 288, 188)).ok()?;
    tree.tree_mut().set_bounds(label_handle, UiRect::new(32, 40, 180, 24)).ok()?;
    logos_ui_graphics::emit(surface, 1, &tree, logos_ui_graphics::UiSceneTheme::DEFAULT).ok()
}

#[cfg(target_os = "none")]
#[inline(always)]
fn yield_now() {
    unsafe {
        core::arch::asm!("mov eax, 1", "int 49", lateout("rax") _, options(preserves_flags));
    }
}

#[cfg(target_os = "none")]
fn idle() -> ! {
    loop {
        yield_now();
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
