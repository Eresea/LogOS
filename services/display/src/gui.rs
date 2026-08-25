use logos_abi::{
    GuiDrawBatch, GuiDrawCommand, GuiDrawKind, GuiRect, GuiStatus, GuiSurfaceOperation,
    GuiSurfaceRequest, GuiSurfaceResponse, MAX_GUI_BATCH_FRAGMENTS, MAX_GUI_DAMAGE_RECTS,
    MAX_GUI_SURFACES, SurfaceHandle,
};

#[derive(Clone, Copy)]
struct SurfaceSlot {
    handle: SurfaceHandle,
    bounds: GuiRect,
    batches: [Option<GuiDrawBatch>; MAX_GUI_BATCH_FRAGMENTS],
    batch_count: u8,
    sequence: u32,
    z_order: i16,
    order: u32,
}

impl SurfaceSlot {
    const EMPTY: Self = Self {
        handle: SurfaceHandle::EMPTY,
        bounds: GuiRect::EMPTY,
        batches: [None; MAX_GUI_BATCH_FRAGMENTS],
        batch_count: 0,
        sequence: 0,
        z_order: 0,
        order: 0,
    };

    const fn occupied(self) -> bool {
        self.handle.is_valid()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuiRegistryError {
    InvalidRequest,
    Stale,
    Unauthorized,
    Capacity,
    Backpressure,
    Malformed,
    NotFound,
}

pub struct GuiSurfaceRegistry {
    slots: [SurfaceSlot; MAX_GUI_SURFACES],
    generations: [u16; MAX_GUI_SURFACES],
    damage: [GuiRect; MAX_GUI_DAMAGE_RECTS],
    damage_count: usize,
    focused: SurfaceHandle,
    order: u32,
}

impl GuiSurfaceRegistry {
    pub const fn new() -> Self {
        Self {
            slots: [SurfaceSlot::EMPTY; MAX_GUI_SURFACES],
            generations: [0; MAX_GUI_SURFACES],
            damage: [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS],
            damage_count: 0,
            focused: SurfaceHandle::EMPTY,
            order: 0,
        }
    }

    pub fn create(
        &mut self,
        owner: u32,
        request: GuiSurfaceRequest,
    ) -> Result<GuiSurfaceResponse, GuiRegistryError> {
        if !request.is_valid() || owner == 0 || request.bounds.is_empty() {
            return Err(GuiRegistryError::InvalidRequest);
        }
        let root_exists = self.slots.iter().any(|slot| slot.occupied() && slot.z_order == 0);
        if matches!(request.operation, GuiSurfaceOperation::CreateRoot) && root_exists {
            return Err(GuiRegistryError::Capacity);
        }
        if !matches!(
            request.operation,
            GuiSurfaceOperation::CreateRoot | GuiSurfaceOperation::CreateModal
        ) {
            return Err(GuiRegistryError::InvalidRequest);
        }
        if matches!(request.operation, GuiSurfaceOperation::CreateModal) && !root_exists {
            return Err(GuiRegistryError::NotFound);
        }
        let Some((slot_index, slot)) =
            self.slots.iter_mut().enumerate().find(|(_, slot)| !slot.occupied())
        else {
            return Err(GuiRegistryError::Capacity);
        };
        let generation = next_generation(&mut self.generations[slot_index]);
        let handle = SurfaceHandle::new(slot_index as u16, generation, owner)
            .ok_or(GuiRegistryError::Capacity)?;
        self.order = self.order.wrapping_add(1).max(1);
        *slot = SurfaceSlot {
            handle,
            bounds: request.bounds,
            batches: [None; MAX_GUI_BATCH_FRAGMENTS],
            batch_count: 0,
            sequence: 0,
            z_order: if matches!(request.operation, GuiSurfaceOperation::CreateRoot) {
                0
            } else {
                request.z_order.max(1)
            },
            order: self.order,
        };
        self.add_damage(request.bounds)?;
        let mut response = GuiSurfaceResponse::new(request, GuiStatus::Ok);
        response.surface = handle;
        Ok(response)
    }

    pub fn update(&mut self, owner: u32, batch: GuiDrawBatch) -> Result<(), GuiRegistryError> {
        if !batch.is_valid() {
            return Err(GuiRegistryError::Malformed);
        }
        let index = self.authorized_index(owner, batch.surface)?;
        let old_bounds = self.slots[index].bounds;
        if self.slots[index].sequence == batch.sequence
            && self.slots[index].batches[..self.slots[index].batch_count as usize]
                .iter()
                .flatten()
                .any(|old| *old == batch)
        {
            return Ok(());
        }
        if self.slots[index].sequence != batch.sequence {
            self.slots[index].batches = [None; MAX_GUI_BATCH_FRAGMENTS];
            self.slots[index].batch_count = 0;
            self.slots[index].sequence = batch.sequence;
        }
        if usize::from(self.slots[index].batch_count) == MAX_GUI_BATCH_FRAGMENTS {
            return Err(GuiRegistryError::Backpressure);
        }
        let slot = &mut self.slots[index];
        slot.batches[usize::from(slot.batch_count)] = Some(batch);
        slot.batch_count += 1;
        self.add_damage(old_bounds)?;
        self.add_damage(batch.damage)?;
        Ok(())
    }

    pub fn invalidate_rect(&mut self, rect: GuiRect) {
        if rect.is_empty() {
            return;
        }
        for index in 0..MAX_GUI_SURFACES {
            if !self.slots[index].occupied() || self.slots[index].batch_count == 0 {
                continue;
            }
            let overlap = intersect(rect, self.slots[index].bounds);
            let _ = self.add_damage(overlap);
        }
    }

    pub fn background_color(&self) -> Option<u32> {
        let mut selected = usize::MAX;
        for index in 0..MAX_GUI_SURFACES {
            let slot = self.slots[index];
            for batch in slot.batches[..slot.batch_count as usize].iter().flatten() {
                let Some(command) = batch.commands[..batch.command_count as usize].first() else {
                    continue;
                };
                if is_surface_fill(*command)
                    && (selected == usize::MAX
                        || (slot.z_order, slot.order)
                            > (self.slots[selected].z_order, self.slots[selected].order))
                {
                    selected = index;
                }
            }
        }
        if selected == usize::MAX {
            None
        } else {
            self.slots[selected]
                .batches
                .iter()
                .flatten()
                .find(|batch| {
                    batch.commands[..batch.command_count as usize]
                        .first()
                        .is_some_and(|command| is_surface_fill(*command))
                })
                .map(|batch| batch.commands[0].color)
        }
    }

    pub fn focus(&mut self, owner: u32, handle: SurfaceHandle) -> Result<(), GuiRegistryError> {
        let index = self.authorized_index(owner, handle)?;
        let old = self.focused;
        self.order = self.order.wrapping_add(1).max(1);
        self.slots[index].order = self.order;
        self.focused = handle;
        if old != handle && old.is_valid() {
            let old_bounds = self.lookup(old)?.bounds;
            self.add_damage(old_bounds)?;
        }
        self.add_damage(self.slots[index].bounds)
    }

    pub fn destroy(&mut self, owner: u32, handle: SurfaceHandle) -> Result<(), GuiRegistryError> {
        let index = self.authorized_index(owner, handle)?;
        let bounds = self.slots[index].bounds;
        if self.focused == handle {
            self.focused = SurfaceHandle::EMPTY;
        }
        self.slots[index] = SurfaceSlot::EMPTY;
        self.add_damage(bounds)
    }

    pub fn focused(&self) -> SurfaceHandle {
        self.focused
    }

    pub fn take_damage(&mut self) -> ([GuiRect; MAX_GUI_DAMAGE_RECTS], usize) {
        let damage = self.damage;
        let count = self.damage_count;
        self.damage = [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS];
        self.damage_count = 0;
        (damage, count)
    }

    pub fn contains(&self, handle: SurfaceHandle) -> bool {
        self.lookup(handle).is_ok()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        framebuffer: &mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        format: super::PixelFormat,
        damage: &[GuiRect; MAX_GUI_DAMAGE_RECTS],
        damage_count: usize,
    ) -> usize {
        if damage_count == 0 {
            return 0;
        }
        let mut order = [usize::MAX; MAX_GUI_SURFACES];
        let mut count = 0;
        while count < MAX_GUI_SURFACES {
            let mut selected = usize::MAX;
            let mut index = 0;
            while index < MAX_GUI_SURFACES {
                if self.slots[index].occupied()
                    && !order[..count].contains(&index)
                    && (selected == usize::MAX || {
                        let current = selected;
                        (self.slots[index].z_order, self.slots[index].order)
                            < (self.slots[current].z_order, self.slots[current].order)
                    })
                {
                    selected = index;
                }
                index += 1;
            }
            if selected == usize::MAX {
                break;
            }
            order[count] = selected;
            count += 1;
        }

        let mut rendered = 0;
        for index in order[..count].iter().copied() {
            let mut clip = self.slots[index].bounds;
            for batch in
                self.slots[index].batches[..self.slots[index].batch_count as usize].iter().flatten()
            {
                for command in batch.commands[..batch.command_count as usize].iter().copied() {
                    if is_surface_fill(command) {
                        continue;
                    }
                    if command.kind == GuiDrawKind::ClipRect {
                        clip = intersect(clip, command_rect(command));
                        continue;
                    }
                    let bounds = command_rect(command);
                    if !damage[..damage_count].iter().any(|rect| touches(*rect, bounds)) {
                        continue;
                    }
                    let command_clip = intersect(clip, bounds);
                    for damage_rect in damage[..damage_count].iter().copied() {
                        let damage_clip = intersect(command_clip, damage_rect);
                        if damage_clip.is_empty() {
                            continue;
                        }
                        rendered += render_command(
                            framebuffer,
                            width,
                            height,
                            stride,
                            format,
                            command,
                            damage_clip,
                            damage,
                            damage_count,
                        );
                    }
                }
            }
        }
        rendered
    }

    fn lookup(&self, handle: SurfaceHandle) -> Result<&SurfaceSlot, GuiRegistryError> {
        let slot = self.slots.get(handle.slot as usize).ok_or(GuiRegistryError::Stale)?;
        if slot.handle != handle {
            return Err(GuiRegistryError::Stale);
        }
        Ok(slot)
    }

    fn authorized_index(
        &self,
        owner: u32,
        handle: SurfaceHandle,
    ) -> Result<usize, GuiRegistryError> {
        let index = handle.slot as usize;
        let slot = self.slots.get(index).ok_or(GuiRegistryError::Stale)?;
        if slot.handle != handle {
            return Err(GuiRegistryError::Stale);
        }
        if slot.handle.owner != owner {
            return Err(GuiRegistryError::Unauthorized);
        }
        Ok(index)
    }

    fn add_damage(&mut self, rect: GuiRect) -> Result<(), GuiRegistryError> {
        if rect.is_empty() {
            return Ok(());
        }
        let mut index = 0;
        while index < self.damage_count {
            if touches(self.damage[index], rect) {
                self.damage[index] = union(self.damage[index], rect);
                return Ok(());
            }
            index += 1;
        }
        if self.damage_count == MAX_GUI_DAMAGE_RECTS {
            return Err(GuiRegistryError::Backpressure);
        }
        self.damage[self.damage_count] = rect;
        self.damage_count += 1;
        Ok(())
    }
}

impl Default for GuiSurfaceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn next_generation(generation: &mut u16) -> u16 {
    *generation = generation.wrapping_add(1).max(1);
    *generation
}

fn touches(left: GuiRect, right: GuiRect) -> bool {
    let left_right = left.x.saturating_add(left.width as i32);
    let right_right = right.x.saturating_add(right.width as i32);
    let left_bottom = left.y.saturating_add(left.height as i32);
    let right_bottom = right.y.saturating_add(right.height as i32);
    left.x <= right_right
        && right.x <= left_right
        && left.y <= right_bottom
        && right.y <= left_bottom
}

fn union(left: GuiRect, right: GuiRect) -> GuiRect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge =
        left.x.saturating_add(left.width as i32).max(right.x.saturating_add(right.width as i32));
    let bottom =
        left.y.saturating_add(left.height as i32).max(right.y.saturating_add(right.height as i32));
    GuiRect::new(x, y, right_edge.saturating_sub(x) as u32, bottom.saturating_sub(y) as u32)
}

fn command_rect(command: GuiDrawCommand) -> GuiRect {
    match command.kind {
        GuiDrawKind::GlyphRun => GuiRect::new(
            command.x,
            command.y,
            u32::from(command.text_len).saturating_mul(super::GLYPH_WIDTH as u32),
            super::GLYPH_HEIGHT as u32,
        ),
        _ => GuiRect::new(command.x, command.y, command.width, command.height),
    }
}

fn is_surface_fill(command: GuiDrawCommand) -> bool {
    command.kind == GuiDrawKind::FillRect
        && GuiRect::new(command.x, command.y, command.width, command.height) == GuiRect::SURFACE
}

fn intersect(left: GuiRect, right: GuiRect) -> GuiRect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge =
        left.x.saturating_add(left.width as i32).min(right.x.saturating_add(right.width as i32));
    let bottom =
        left.y.saturating_add(left.height as i32).min(right.y.saturating_add(right.height as i32));
    if right_edge <= x || bottom <= y {
        GuiRect::EMPTY
    } else {
        GuiRect::new(x, y, (right_edge - x) as u32, (bottom - y) as u32)
    }
}

#[allow(clippy::too_many_arguments)]
fn render_command(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    command: GuiDrawCommand,
    clip: GuiRect,
    damage: &[GuiRect; MAX_GUI_DAMAGE_RECTS],
    damage_count: usize,
) -> usize {
    if clip.is_empty() {
        return 0;
    }
    match command.kind {
        GuiDrawKind::FillRect => {
            let left = clip.x.max(0).min(width as i32) as usize;
            let top = clip.y.max(0).min(height as i32) as usize;
            let right = clip.x.saturating_add(clip.width as i32).max(0).min(width as i32) as usize;
            let bottom =
                clip.y.saturating_add(clip.height as i32).max(0).min(height as i32) as usize;
            if left >= right || top >= bottom {
                return 0;
            }
            let pixel = super::pixel_bytes(command.color, format);
            for y in top..bottom {
                let start = y * stride + left * 4;
                let end = y * stride + right * 4;
                for chunk in framebuffer[start..end].chunks_exact_mut(4) {
                    chunk.copy_from_slice(&pixel);
                }
            }
            (right - left) * (bottom - top)
        }
        GuiDrawKind::StrokeRect => {
            let edge = command.auxiliary.max(1).min(command.width.min(command.height));
            let mut rendered = 0;
            for y in clip.y..clip.y.saturating_add(clip.height as i32) {
                for x in clip.x..clip.x.saturating_add(clip.width as i32) {
                    let local_x = x.saturating_sub(command.x) as u32;
                    let local_y = y.saturating_sub(command.y) as u32;
                    if local_x < edge
                        || local_y < edge
                        || local_x >= command.width.saturating_sub(edge)
                        || local_y >= command.height.saturating_sub(edge)
                    {
                        rendered += plot(
                            framebuffer,
                            width,
                            height,
                            stride,
                            format,
                            x,
                            y,
                            command.color,
                            255,
                            damage,
                            damage_count,
                        ) as usize;
                    }
                }
            }
            rendered
        }
        GuiDrawKind::Line => {
            let mut rendered = 0;
            let mut x0 = command.x;
            let mut y0 = command.y;
            let x1 = command.x.saturating_add(command.width as i32);
            let y1 = command.y.saturating_add(command.height as i32);
            let dx = (x1 - x0).abs();
            let sx = if x0 < x1 { 1 } else { -1 };
            let dy = -(y1 - y0).abs();
            let sy = if y0 < y1 { 1 } else { -1 };
            let mut error = dx + dy;
            loop {
                if clip.contains(x0, y0) {
                    rendered += plot(
                        framebuffer,
                        width,
                        height,
                        stride,
                        format,
                        x0,
                        y0,
                        command.color,
                        255,
                        damage,
                        damage_count,
                    ) as usize;
                }
                if x0 == x1 && y0 == y1 {
                    break;
                }
                let doubled = error * 2;
                if doubled >= dy {
                    error += dy;
                    x0 += sx;
                }
                if doubled <= dx {
                    error += dx;
                    y0 += sy;
                }
            }
            rendered
        }
        GuiDrawKind::ClipRect => 0,
        GuiDrawKind::GlyphRun => {
            let mut rendered = 0;
            for (index, byte) in
                command.text[..command.text_len as usize].iter().copied().enumerate()
            {
                let glyph = super::embedded_glyph(u32::from(byte));
                let base_x = command.x + (index * super::GLYPH_WIDTH) as i32;
                for glyph_y in 0..super::GLYPH_HEIGHT {
                    for glyph_x in 0..super::GLYPH_WIDTH {
                        let coverage = glyph.rows[glyph_y][glyph_x];
                        if coverage != 0 {
                            let x = base_x + glyph_x as i32;
                            let y = command.y + glyph_y as i32;
                            if clip.contains(x, y) {
                                rendered += plot(
                                    framebuffer,
                                    width,
                                    height,
                                    stride,
                                    format,
                                    x,
                                    y,
                                    command.color,
                                    coverage,
                                    damage,
                                    damage_count,
                                ) as usize;
                            }
                        }
                    }
                }
            }
            rendered
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn plot(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    x: i32,
    y: i32,
    color: u32,
    coverage: u8,
    damage: &[GuiRect; MAX_GUI_DAMAGE_RECTS],
    damage_count: usize,
) -> bool {
    if x < 0
        || y < 0
        || x as usize >= width
        || y as usize >= height
        || !damage[..damage_count].iter().any(|rect| rect.contains(x, y))
    {
        return false;
    }
    let offset = y as usize * stride + x as usize * 4;
    if offset + 4 > framebuffer.len() {
        return false;
    }
    let background = match format {
        super::PixelFormat::Rgb8 => {
            u32::from(framebuffer[offset]) << 16
                | u32::from(framebuffer[offset + 1]) << 8
                | u32::from(framebuffer[offset + 2])
        }
        super::PixelFormat::Bgr8 => {
            u32::from(framebuffer[offset + 2]) << 16
                | u32::from(framebuffer[offset + 1]) << 8
                | u32::from(framebuffer[offset])
        }
    };
    framebuffer[offset..offset + 4].copy_from_slice(&super::pixel_bytes(
        super::blend_color(background, color, coverage),
        format,
    ));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::GuiDrawCommand;

    fn request(operation: GuiSurfaceOperation, id: u32, bounds: GuiRect) -> GuiSurfaceRequest {
        let mut request = GuiSurfaceRequest::new(operation, id);
        request.bounds = bounds;
        request.z_order = 1;
        request
    }

    #[test]
    fn root_modal_focus_and_generation_reuse_are_bounded() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 100, 100)))
            .unwrap()
            .surface;
        let modal = registry
            .create(7, request(GuiSurfaceOperation::CreateModal, 2, GuiRect::new(10, 10, 20, 20)))
            .unwrap()
            .surface;
        assert_ne!(root, modal);
        assert_eq!(registry.focus(7, modal), Ok(()));
        assert_eq!(registry.focused(), modal);
        registry.take_damage();
        registry.destroy(7, modal).unwrap();
        let replacement = registry
            .create(7, request(GuiSurfaceOperation::CreateModal, 3, GuiRect::new(10, 10, 20, 20)))
            .unwrap()
            .surface;
        assert_eq!(replacement.slot, modal.slot);
        assert_ne!(replacement.generation, modal.generation);
        assert!(!registry.contains(modal));
    }

    #[test]
    fn stale_and_unauthorized_handles_are_rejected() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 10, 10)))
            .unwrap()
            .surface;
        assert_eq!(registry.destroy(8, root), Err(GuiRegistryError::Unauthorized));
        let stale = SurfaceHandle { generation: root.generation.wrapping_add(1), ..root };
        assert_eq!(registry.destroy(7, stale), Err(GuiRegistryError::Stale));
    }

    #[test]
    fn damage_is_coalesced_and_identical_batches_do_not_redraw() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 10, 10)))
            .unwrap()
            .surface;
        registry.take_damage();
        let mut batch = GuiDrawBatch::new(root, 1, GuiRect::new(1, 1, 2, 2));
        assert!(batch.push(GuiDrawCommand::fill_rect(GuiRect::new(1, 1, 2, 2), 0xffffff)));
        registry.update(7, batch).unwrap();
        registry.update(7, batch).unwrap();
        let (_, count) = registry.take_damage();
        assert_eq!(count, 1);
    }

    #[test]
    fn same_sequence_batches_accumulate_with_a_bounded_fragment_limit() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 16, 16)))
            .unwrap()
            .surface;
        registry.take_damage();
        for index in 0..MAX_GUI_BATCH_FRAGMENTS {
            let mut batch = GuiDrawBatch::new(root, 4, GuiRect::new(index as i32, 0, 1, 1));
            if index + 1 < MAX_GUI_BATCH_FRAGMENTS {
                batch.flags = logos_abi::GUI_DRAW_FLAG_MORE;
            }
            assert!(
                batch
                    .push(
                        GuiDrawCommand::fill_rect(GuiRect::new(index as i32, 0, 1, 1), 0xffffff,)
                    )
            );
            registry.update(7, batch).unwrap();
        }
        let overflow = GuiDrawBatch::new(root, 4, GuiRect::new(8, 0, 1, 1));
        assert_eq!(registry.update(7, overflow), Err(GuiRegistryError::Backpressure));
    }

    #[test]
    fn malformed_commands_and_capacity_are_rejected() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 10, 10)))
            .unwrap()
            .surface;
        let mut batch = GuiDrawBatch::new(root, 1, GuiRect::new(0, 0, 1, 1));
        batch.command_count = 1;
        assert_eq!(registry.update(7, batch), Err(GuiRegistryError::Malformed));
        for index in 0..MAX_GUI_SURFACES {
            if index == root.slot as usize {
                continue;
            }
            let result = registry.create(
                7,
                request(
                    GuiSurfaceOperation::CreateModal,
                    index as u32 + 2,
                    GuiRect::new(index as i32, 0, 1, 1),
                ),
            );
            assert!(result.is_ok());
        }
        assert_eq!(
            registry
                .create(7, request(GuiSurfaceOperation::CreateModal, 99, GuiRect::new(0, 0, 1, 1))),
            Err(GuiRegistryError::Capacity)
        );
    }
}
