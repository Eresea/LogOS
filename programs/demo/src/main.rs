#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

#[cfg(target_os = "none")]
use logos_abi::{AtriumApp, SurfaceHandle};
#[cfg(target_os = "none")]
use logos_program::{ProgramClient, SurfaceEvent};
#[cfg(target_os = "none")]
use logos_ui::{UiBlueprint, UiNodeKind, UiRect, UiText};
#[cfg(target_os = "none")]
use logos_ui_graphics::UiComponentTree;

#[cfg(target_os = "none")]
static mut UI_SCENE_PUBLISHER: logos_ui_graphics::UiScenePublisher =
    logos_ui_graphics::UiScenePublisher::new();

#[cfg(target_os = "none")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut client = match unsafe { ProgramClient::from_fixed_bootstrap() } {
        Ok(client) => client,
        Err(_) => idle(),
    };
    let _ = client.request_surface(AtriumApp::Calculator);
    let mut frame = 0u32;
    let mut scene_pending = false;
    loop {
        let _ = client.retry_surface_request();
        match client.poll_surface() {
            Ok(Some(SurfaceEvent::Created(_))) => {
                frame = frame.wrapping_add(1).max(1);
                scene_pending = true;
            }
            Ok(Some(SurfaceEvent::Revoked(_))) => unsafe {
                (*core::ptr::addr_of_mut!(UI_SCENE_PUBLISHER)).reset();
                scene_pending = false;
                let _ = client.request_surface(AtriumApp::Calculator);
            },
            Ok(None) | Err(_) => {}
        }
        if scene_pending && client.has_surface() {
            if let Some(tree) = demo_tree() {
                let publisher = unsafe { &mut *core::ptr::addr_of_mut!(UI_SCENE_PUBLISHER) };
                if client.send_scene(publisher, frame, &tree).is_ok() {
                    scene_pending = false;
                }
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
fn demo_tree() -> Option<UiComponentTree> {
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
    Some(tree)
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
