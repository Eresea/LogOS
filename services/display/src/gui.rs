use logos_abi::{
    GUI_SURFACE_FLAG_TERMINAL, GUI_TEXT_FLAG_LIGHT, GuiDrawBatch, GuiDrawCommand, GuiDrawKind,
    GuiNodeOperation, GuiRect, GuiSceneOp, GuiStatus, GuiSurfaceOperation, GuiSurfaceRequest,
    GuiSurfaceResponse, MAX_GUI_BATCH_FRAGMENTS, MAX_GUI_DAMAGE_RECTS, MAX_GUI_NODES,
    MAX_GUI_SURFACES, SurfaceHandle,
};

#[derive(Clone, Copy)]
struct RenderNode {
    id: u32,
    command: GuiDrawCommand,
}

impl RenderNode {
    const EMPTY: Option<Self> = None;
}

const MAX_GUI_PLAN_COMMANDS: usize = MAX_GUI_SURFACES * MAX_GUI_NODES;
const MAX_GUI_PLAN_OCCLUDERS: usize = MAX_GUI_PLAN_COMMANDS;

#[derive(Clone, Copy)]
struct RenderPlanEntry {
    command: GuiDrawCommand,
    clip: GuiRect,
    occluder_start: u16,
    occluder_count: u16,
}

struct RenderPlan {
    entries: [Option<RenderPlanEntry>; MAX_GUI_PLAN_COMMANDS],
    entry_count: usize,
    occluders: [GuiRect; MAX_GUI_PLAN_OCCLUDERS],
    occluder_count: usize,
    valid: bool,
}

impl RenderPlan {
    const fn new() -> Self {
        Self {
            entries: [None; MAX_GUI_PLAN_COMMANDS],
            entry_count: 0,
            occluders: [GuiRect::EMPTY; MAX_GUI_PLAN_OCCLUDERS],
            occluder_count: 0,
            valid: false,
        }
    }
}

pub trait GuiRenderBackend {
    fn draw(&mut self, command: GuiDrawCommand, clip: GuiRect) -> usize;
}

struct SoftwareRenderBackend<'framebuffer, 'glyph> {
    framebuffer: &'framebuffer mut [u8],
    glyph_cache: &'glyph mut super::GlyphCache,
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
}

impl GuiRenderBackend for SoftwareRenderBackend<'_, '_> {
    fn draw(&mut self, command: GuiDrawCommand, clip: GuiRect) -> usize {
        render_command(
            self.framebuffer,
            self.width,
            self.height,
            self.stride,
            self.format,
            self.glyph_cache,
            command,
            clip,
        )
    }
}

#[derive(Clone, Copy)]
struct SurfaceSlot {
    handle: SurfaceHandle,
    bounds: GuiRect,
    batches: [Option<GuiDrawBatch>; MAX_GUI_BATCH_FRAGMENTS],
    batch_count: u8,
    sequence: u32,
    z_order: i16,
    order: u32,
    terminal: bool,
    active_nodes: [Option<RenderNode>; MAX_GUI_NODES],
    staged_nodes: [Option<RenderNode>; MAX_GUI_NODES],
    active_node_count: u8,
    staged_node_count: u8,
    active_frame: u32,
    staged_frame: u32,
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
        terminal: false,
        active_nodes: [RenderNode::EMPTY; MAX_GUI_NODES],
        staged_nodes: [RenderNode::EMPTY; MAX_GUI_NODES],
        active_node_count: 0,
        staged_node_count: 0,
        active_frame: 0,
        staged_frame: 0,
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
    plan: RenderPlan,
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
            plan: RenderPlan::new(),
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
        if request.flags & GUI_SURFACE_FLAG_TERMINAL != 0
            && !matches!(request.operation, GuiSurfaceOperation::CreateModal)
        {
            return Err(GuiRegistryError::InvalidRequest);
        }
        if request.flags & GUI_SURFACE_FLAG_TERMINAL != 0
            && self.slots.iter().any(|slot| slot.occupied() && slot.terminal)
        {
            return Err(GuiRegistryError::Capacity);
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
            terminal: request.flags & GUI_SURFACE_FLAG_TERMINAL != 0,
            active_nodes: [RenderNode::EMPTY; MAX_GUI_NODES],
            staged_nodes: [RenderNode::EMPTY; MAX_GUI_NODES],
            active_node_count: 0,
            staged_node_count: 0,
            active_frame: 0,
            staged_frame: 0,
        };
        self.plan.valid = false;
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
        if self.slots[index].sequence == batch.sequence
            && self.slots[index].batches[..self.slots[index].batch_count as usize]
                .iter()
                .flatten()
                .any(|old| *old == batch)
        {
            return Ok(());
        }
        if self.slots[index].sequence != batch.sequence {
            self.damage_legacy_nodes(index)?;
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
        self.plan.valid = false;
        self.add_damage(batch.damage)?;
        Ok(())
    }

    fn damage_legacy_nodes(&mut self, index: usize) -> Result<(), GuiRegistryError> {
        let batches = self.slots[index].batches;
        let batch_count = self.slots[index].batch_count as usize;
        for batch in batches[..batch_count].iter().flatten() {
            for command in batch.commands[..batch.command_count as usize].iter().copied() {
                self.add_damage(command_rect(command))?;
            }
        }
        Ok(())
    }

    pub fn apply_scene_op(&mut self, owner: u32, op: GuiSceneOp) -> Result<(), GuiRegistryError> {
        if !op.is_valid() {
            return Err(GuiRegistryError::Malformed);
        }
        let index = self.authorized_index(owner, op.surface)?;
        if is_older_frame(op.frame, self.slots[index].active_frame)
            || (self.slots[index].staged_frame != 0
                && is_older_frame(op.frame, self.slots[index].staged_frame))
        {
            return Err(GuiRegistryError::Stale);
        }
        if self.slots[index].staged_frame != op.frame {
            self.slots[index].staged_nodes = self.slots[index].active_nodes;
            self.slots[index].staged_node_count = self.slots[index].active_node_count;
            self.slots[index].staged_frame = op.frame;
        }
        match op.operation {
            GuiNodeOperation::Upsert => {
                let node = RenderNode { id: op.node_id, command: op.command };
                if let Some(existing) = self.slots[index].staged_nodes[..MAX_GUI_NODES]
                    .iter_mut()
                    .flatten()
                    .find(|existing| existing.id == op.node_id)
                {
                    *existing = node;
                } else {
                    let count = usize::from(self.slots[index].staged_node_count);
                    if count == MAX_GUI_NODES {
                        return Err(GuiRegistryError::Backpressure);
                    }
                    self.slots[index].staged_nodes[count] = Some(node);
                    self.slots[index].staged_node_count += 1;
                }
            }
            GuiNodeOperation::Remove => {
                let Some(node_index) = self.slots[index].staged_nodes[..MAX_GUI_NODES]
                    .iter()
                    .position(|node| node.is_some_and(|node| node.id == op.node_id))
                else {
                    return if op.flags & logos_abi::GUI_DRAW_FLAG_MORE == 0 {
                        self.publish_scene(index)
                    } else {
                        Ok(())
                    };
                };
                self.slots[index].staged_nodes[node_index..MAX_GUI_NODES].rotate_left(1);
                self.slots[index].staged_nodes[MAX_GUI_NODES - 1] = None;
                self.slots[index].staged_node_count -= 1;
            }
            GuiNodeOperation::Clear => {
                self.slots[index].staged_nodes = [RenderNode::EMPTY; MAX_GUI_NODES];
                self.slots[index].staged_node_count = 0;
            }
            GuiNodeOperation::Commit => {}
        }
        if op.flags & logos_abi::GUI_DRAW_FLAG_MORE == 0 {
            self.publish_scene(index)
        } else {
            Ok(())
        }
    }

    fn publish_scene(&mut self, index: usize) -> Result<(), GuiRegistryError> {
        let old_nodes = self.slots[index].active_nodes;
        let old_count = usize::from(self.slots[index].active_node_count);
        let new_nodes = self.slots[index].staged_nodes;
        let new_count = usize::from(self.slots[index].staged_node_count);
        for old in old_nodes[..old_count].iter().flatten() {
            let changed =
                new_nodes[..new_count].iter().flatten().find(|new| new.id == old.id).is_none_or(
                    |new| {
                        command_rect(new.command) != command_rect(old.command)
                            || new.command != old.command
                    },
                );
            if changed {
                self.add_damage(command_rect(old.command))?;
            }
        }
        for new in new_nodes[..new_count].iter().flatten() {
            let changed =
                old_nodes[..old_count].iter().flatten().find(|old| old.id == new.id).is_none_or(
                    |old| {
                        command_rect(old.command) != command_rect(new.command)
                            || old.command != new.command
                    },
                );
            if changed {
                self.add_damage(command_rect(new.command))?;
            }
        }
        self.slots[index].active_nodes = new_nodes;
        self.slots[index].active_node_count = self.slots[index].staged_node_count;
        self.slots[index].active_frame = self.slots[index].staged_frame;
        self.slots[index].staged_frame = 0;
        self.plan.valid = false;
        Ok(())
    }

    pub fn active_frame(&self, handle: SurfaceHandle) -> Option<u32> {
        self.lookup(handle).ok().map(|slot| slot.active_frame)
    }

    pub fn invalidate_rect(&mut self, rect: GuiRect) {
        if rect.is_empty() {
            return;
        }
        for index in 0..MAX_GUI_SURFACES {
            if !self.slots[index].occupied() {
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
            let command = if slot.active_frame != 0 {
                slot.active_nodes[..slot.active_node_count as usize]
                    .iter()
                    .flatten()
                    .map(|node| node.command)
                    .find(|command| is_surface_fill(*command))
            } else {
                slot.batches[..slot.batch_count as usize]
                    .iter()
                    .flatten()
                    .filter_map(|batch| batch.commands[..batch.command_count as usize].first())
                    .copied()
                    .find(|command| is_surface_fill(*command))
            };
            if command.is_some()
                && (selected == usize::MAX
                    || (slot.z_order, slot.order)
                        > (self.slots[selected].z_order, self.slots[selected].order))
            {
                selected = index;
            }
        }
        if selected == usize::MAX {
            None
        } else {
            let slot = self.slots[selected];
            if slot.active_frame != 0 {
                slot.active_nodes[..slot.active_node_count as usize]
                    .iter()
                    .flatten()
                    .map(|node| node.command)
                    .find(|command| is_surface_fill(*command))
                    .map(|command| command.color)
            } else {
                slot.batches.iter().flatten().find_map(|batch| {
                    batch.commands[..batch.command_count as usize]
                        .first()
                        .filter(|command| is_surface_fill(**command))
                        .map(|command| command.color)
                })
            }
        }
    }

    pub fn focus(&mut self, owner: u32, handle: SurfaceHandle) -> Result<(), GuiRegistryError> {
        let index = self.authorized_index(owner, handle)?;
        if self.focused == handle {
            return Ok(());
        }
        let old = self.focused;
        self.order = self.order.wrapping_add(1).max(1);
        self.slots[index].order = self.order;
        self.focused = handle;
        self.plan.valid = false;
        if old != handle && old.is_valid() {
            let old_bounds = self.lookup(old)?.bounds;
            self.add_damage(old_bounds)?;
        }
        self.add_damage(self.slots[index].bounds)
    }

    pub fn set_bounds(
        &mut self,
        owner: u32,
        handle: SurfaceHandle,
        bounds: GuiRect,
    ) -> Result<(), GuiRegistryError> {
        if bounds.is_empty() {
            return Err(GuiRegistryError::InvalidRequest);
        }
        let index = self.authorized_index(owner, handle)?;
        let old = self.slots[index].bounds;
        if old == bounds {
            return Ok(());
        }
        self.slots[index].bounds = bounds;
        self.plan.valid = false;
        self.add_damage(old)?;
        self.add_damage(bounds)
    }

    pub fn destroy(&mut self, owner: u32, handle: SurfaceHandle) -> Result<(), GuiRegistryError> {
        let index = self.authorized_index(owner, handle)?;
        let bounds = self.slots[index].bounds;
        if self.focused == handle {
            self.focused = SurfaceHandle::EMPTY;
        }
        self.slots[index] = SurfaceSlot::EMPTY;
        self.plan.valid = false;
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

    pub const fn has_damage(&self) -> bool {
        self.damage_count != 0
    }

    pub fn contains(&self, handle: SurfaceHandle) -> bool {
        self.lookup(handle).is_ok()
    }

    pub fn terminal_bounds(&self) -> Option<GuiRect> {
        self.terminal_surface().map(|(_, bounds)| bounds)
    }

    pub fn terminal_surface(&self) -> Option<(SurfaceHandle, GuiRect)> {
        self.slots
            .iter()
            .find(|slot| slot.occupied() && slot.terminal)
            .map(|slot| (slot.handle, slot.bounds))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        glyph_cache: &mut super::GlyphCache,
        framebuffer: &mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        format: super::PixelFormat,
        damage: &[GuiRect; MAX_GUI_DAMAGE_RECTS],
        damage_count: usize,
    ) -> usize {
        let mut backend =
            SoftwareRenderBackend { framebuffer, glyph_cache, width, height, stride, format };
        self.compose(&mut backend, damage, damage_count)
    }

    pub fn compose<B: GuiRenderBackend>(
        &mut self,
        backend: &mut B,
        damage: &[GuiRect; MAX_GUI_DAMAGE_RECTS],
        damage_count: usize,
    ) -> usize {
        if damage_count == 0 {
            return 0;
        }
        self.ensure_plan();
        let mut rendered = 0;
        for entry in self.plan.entries[..self.plan.entry_count].iter().flatten() {
            let start = usize::from(entry.occluder_start);
            let end = start + usize::from(entry.occluder_count);
            let mut clip = entry.clip;
            rendered += render_one(
                backend,
                entry.command,
                &mut clip,
                damage,
                damage_count,
                &self.plan.occluders[start..end],
                usize::from(entry.occluder_count),
            );
        }
        rendered
    }

    fn ensure_plan(&mut self) {
        if self.plan.valid {
            return;
        }
        self.plan = RenderPlan::new();
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

        for index in order[..count].iter().copied() {
            let mut clip = self.slots[index].bounds;
            if self.slots[index].active_frame != 0 {
                let node_count = self.slots[index].active_node_count as usize;
                let nodes = self.slots[index].active_nodes;
                for node in nodes[..node_count].iter().flatten() {
                    let command = node.command;
                    self.append_plan_entry(command, &mut clip);
                }
            } else {
                let batch_count = self.slots[index].batch_count as usize;
                let batches = self.slots[index].batches;
                for batch in batches[..batch_count].iter().flatten() {
                    for command in batch.commands[..batch.command_count as usize].iter().copied() {
                        self.append_plan_entry(command, &mut clip);
                    }
                }
            }
        }
        let occluder_count = self.plan.occluder_count;
        for entry in self.plan.entries[..self.plan.entry_count].iter_mut().flatten() {
            let start = usize::from(entry.occluder_start);
            entry.occluder_count = occluder_count.saturating_sub(start) as u16;
        }
        self.plan.valid = true;
    }

    fn append_plan_entry(&mut self, command: GuiDrawCommand, clip: &mut GuiRect) {
        if command.kind == GuiDrawKind::ClipRect {
            *clip = intersect(*clip, command_rect(command));
            return;
        }
        if self.plan.entry_count == MAX_GUI_PLAN_COMMANDS {
            return;
        }
        let occluder_start = self.plan.occluder_count + usize::from(is_opaque_occluder(command));
        if is_opaque_occluder(command) && self.plan.occluder_count < MAX_GUI_PLAN_OCCLUDERS {
            self.plan.occluders[self.plan.occluder_count] = command_rect(command);
            self.plan.occluder_count += 1;
        }
        self.plan.entries[self.plan.entry_count] = Some(RenderPlanEntry {
            command,
            clip: *clip,
            occluder_start: occluder_start as u16,
            occluder_count: 0,
        });
        self.plan.entry_count += 1;
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

fn render_one<B: GuiRenderBackend>(
    backend: &mut B,
    command: GuiDrawCommand,
    clip: &mut GuiRect,
    damage: &[GuiRect; MAX_GUI_DAMAGE_RECTS],
    damage_count: usize,
    occluders: &[GuiRect],
    occluder_count: usize,
) -> usize {
    if is_surface_fill(command) {
        return 0;
    }
    if command.kind == GuiDrawKind::ClipRect {
        *clip = intersect(*clip, command_rect(command));
        return 0;
    }
    let bounds = command_rect(command);
    if !damage[..damage_count].iter().any(|rect| touches(*rect, bounds)) {
        return 0;
    }
    let command_clip = intersect(*clip, bounds);
    let mut rendered = 0;
    if occluder_count == 0 {
        for damage_rect in damage[..damage_count].iter().copied() {
            let damage_clip = intersect(command_clip, damage_rect);
            if !damage_clip.is_empty() {
                rendered += backend.draw(command, damage_clip);
            }
        }
        return rendered;
    }
    for damage_rect in damage[..damage_count].iter().copied() {
        let damage_clip = intersect(command_clip, damage_rect);
        if damage_clip.is_empty() {
            continue;
        }
        let mut visible = [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS * 4];
        let mut next = [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS * 4];
        visible[0] = damage_clip;
        let mut visible_count = 1;
        for occluder in occluders[..occluder_count].iter().copied() {
            let mut next_count = 0;
            for rect in visible[..visible_count].iter().copied() {
                let mut pieces = [GuiRect::EMPTY; 4];
                let piece_count = subtract_rect(rect, occluder, &mut pieces);
                for piece in pieces[..piece_count].iter().copied() {
                    if next_count == next.len() {
                        break;
                    }
                    next[next_count] = piece;
                    next_count += 1;
                }
            }
            core::mem::swap(&mut visible, &mut next);
            visible_count = next_count;
            if visible_count == 0 {
                break;
            }
        }
        for damage_clip in visible[..visible_count].iter().copied() {
            rendered += backend.draw(command, damage_clip);
        }
    }
    rendered
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

fn is_older_frame(frame: u32, current: u32) -> bool {
    frame != current && current.wrapping_sub(frame) < (1 << 31)
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
        GuiDrawKind::Line => expand_rect(
            GuiRect::new(command.x, command.y, command.width, command.height),
            i32::from(command.line_width()),
        ),
        GuiDrawKind::Shadow => shadow_bounds(command),
        _ => GuiRect::new(command.x, command.y, command.width, command.height),
    }
}

fn expand_rect(rect: GuiRect, padding: i32) -> GuiRect {
    if padding <= 0 {
        return rect;
    }
    let padding = padding as u32;
    GuiRect::new(
        rect.x.saturating_sub(padding as i32),
        rect.y.saturating_sub(padding as i32),
        rect.width.saturating_add(padding.saturating_mul(2)),
        rect.height.saturating_add(padding.saturating_mul(2)),
    )
}

fn shadow_bounds(command: GuiDrawCommand) -> GuiRect {
    let blur = u32::from(command.shadow_blur());
    let x =
        command.x.saturating_add(i32::from(command.shadow_offset_x())).saturating_sub(blur as i32);
    let y =
        command.y.saturating_add(i32::from(command.shadow_offset_y())).saturating_sub(blur as i32);
    GuiRect::new(
        x,
        y,
        command.width.saturating_add(blur.saturating_mul(2)),
        command.height.saturating_add(blur.saturating_mul(2)),
    )
}

fn is_surface_fill(command: GuiDrawCommand) -> bool {
    command.kind == GuiDrawKind::FillRect
        && GuiRect::new(command.x, command.y, command.width, command.height) == GuiRect::SURFACE
}

fn is_opaque_occluder(command: GuiDrawCommand) -> bool {
    matches!(command.kind, GuiDrawKind::FillRect | GuiDrawKind::FillRoundedRect)
        && color_alpha(command.color) == u8::MAX
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

fn subtract_rect(rect: GuiRect, cut: GuiRect, pieces: &mut [GuiRect; 4]) -> usize {
    let overlap = intersect(rect, cut);
    if overlap.is_empty() {
        pieces[0] = rect;
        return 1;
    }
    let rect_right = rect.x.saturating_add(rect.width as i32);
    let rect_bottom = rect.y.saturating_add(rect.height as i32);
    let overlap_right = overlap.x.saturating_add(overlap.width as i32);
    let overlap_bottom = overlap.y.saturating_add(overlap.height as i32);
    let mut count = 0;
    if rect.y < overlap.y {
        pieces[count] = GuiRect::new(rect.x, rect.y, rect.width, (overlap.y - rect.y) as u32);
        count += 1;
    }
    if overlap_bottom < rect_bottom {
        pieces[count] =
            GuiRect::new(rect.x, overlap_bottom, rect.width, (rect_bottom - overlap_bottom) as u32);
        count += 1;
    }
    if rect.x < overlap.x {
        pieces[count] =
            GuiRect::new(rect.x, overlap.y, (overlap.x - rect.x) as u32, overlap.height);
        count += 1;
    }
    if overlap_right < rect_right {
        pieces[count] = GuiRect::new(
            overlap_right,
            overlap.y,
            (rect_right - overlap_right) as u32,
            overlap.height,
        );
        count += 1;
    }
    count
}

#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn render_command(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    glyph_cache: &mut super::GlyphCache,
    command: GuiDrawCommand,
    clip: GuiRect,
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
            if command.color_alpha() != u8::MAX {
                let mut rendered = 0;
                for y in top..bottom {
                    for x in left..right {
                        rendered += plot(
                            framebuffer,
                            width,
                            height,
                            stride,
                            format,
                            x as i32,
                            y as i32,
                            command.color,
                            u8::MAX,
                        ) as usize;
                    }
                }
                return rendered;
            }
            let pixel = super::pixel_bytes(command.color, format);
            for y in top..bottom {
                let start = y * stride + left * 4;
                let end = y * stride + right * 4;
                super::fill_row(&mut framebuffer[start..end], pixel);
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
                        ) as usize;
                    }
                }
            }
            rendered
        }
        GuiDrawKind::Line
        | GuiDrawKind::FillRoundedRect
        | GuiDrawKind::StrokeRoundedRect
        | GuiDrawKind::Shadow => {
            render_modern(framebuffer, width, height, stride, format, command, clip)
        }
        GuiDrawKind::ClipRect => 0,
        GuiDrawKind::GlyphRun => {
            let mut rendered = 0;
            let packed = super::pixel_bytes(command.color, format);
            let clip_left = clip.x.max(0) as usize;
            let clip_top = clip.y.max(0) as usize;
            let clip_right =
                clip.x.saturating_add(clip.width as i32).max(0).min(width as i32) as usize;
            let clip_bottom =
                clip.y.saturating_add(clip.height as i32).max(0).min(height as i32) as usize;
            for (index, byte) in
                command.text[..command.text_len as usize].iter().copied().enumerate()
            {
                let glyph = glyph_cache.get(u32::from(byte));
                let base_x = command.x + (index * super::GLYPH_WIDTH) as i32;
                let first_x =
                    (clip_left as i32 - base_x).max(0).min(super::GLYPH_WIDTH as i32) as usize;
                let last_x =
                    (clip_right as i32 - base_x).max(0).min(super::GLYPH_WIDTH as i32) as usize;
                let first_y =
                    (clip_top as i32 - command.y).max(0).min(super::GLYPH_HEIGHT as i32) as usize;
                let last_y = (clip_bottom as i32 - command.y).max(0).min(super::GLYPH_HEIGHT as i32)
                    as usize;
                for glyph_y in first_y..last_y {
                    for glyph_x in first_x..last_x {
                        let coverage = glyph.rows[glyph_y][glyph_x];
                        let coverage = if command.auxiliary & GUI_TEXT_FLAG_LIGHT != 0 {
                            (u16::from(coverage) * 3 / 4) as u8
                        } else {
                            coverage
                        };
                        if coverage != 0 {
                            let x = base_x + glyph_x as i32;
                            let y = command.y + glyph_y as i32;
                            rendered += plot_packed(
                                framebuffer,
                                width,
                                height,
                                stride,
                                format,
                                x,
                                y,
                                command.color,
                                packed,
                                coverage,
                            ) as usize;
                        }
                    }
                }
            }
            rendered
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn render_modern(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    command: GuiDrawCommand,
    clip: GuiRect,
) -> usize {
    let bounds = GuiRect::new(command.x, command.y, command.width, command.height);
    let inner = rounded_stroke_inner(command, bounds);
    let clip = intersect(clip, GuiRect::new(0, 0, width as u32, height as u32));
    match command.kind {
        GuiDrawKind::FillRoundedRect => render_rounded_fill(
            framebuffer,
            width,
            height,
            stride,
            format,
            bounds,
            command.corner_radius(),
            command.color,
            clip,
        ),
        GuiDrawKind::StrokeRoundedRect => render_rounded_stroke(
            framebuffer,
            width,
            height,
            stride,
            format,
            bounds,
            command.corner_radius(),
            command.stroke_width(),
            command.color,
            clip,
            inner,
        ),
        GuiDrawKind::Line => render_line(framebuffer, width, height, stride, format, command, clip),
        GuiDrawKind::Shadow => {
            let blur = command.shadow_blur();
            let shifted_shadow = GuiRect::new(
                bounds.x.saturating_add(i32::from(command.shadow_offset_x())),
                bounds.y.saturating_add(i32::from(command.shadow_offset_y())),
                bounds.width,
                bounds.height,
            );
            let mut shadow_shapes = [GuiRect::EMPTY; 5];
            let mut distance = 0;
            while distance <= blur {
                shadow_shapes[distance as usize] = expand_rect(shifted_shadow, i32::from(distance));
                distance += 1;
            }
            render_shadow(
                framebuffer,
                width,
                height,
                stride,
                format,
                command,
                clip,
                &shadow_shapes,
                blur,
            )
        }
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_rounded_fill(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    bounds: GuiRect,
    radius: u8,
    color: u32,
    clip: GuiRect,
) -> usize {
    if clip.is_empty() {
        return 0;
    }
    let left = bounds.x;
    let right = bounds.x.saturating_add(bounds.width as i32);
    let top = bounds.y;
    let radius = i32::from(radius);
    let clip_right = clip.x.saturating_add(clip.width as i32);
    let mut rendered = 0;
    for y in clip.y..clip.y.saturating_add(clip.height as i32) {
        let local_y = y.saturating_sub(top);
        if radius == 0 || (local_y >= radius && local_y < bounds.height as i32 - radius) {
            rendered += fill_span(
                framebuffer,
                width,
                height,
                stride,
                format,
                clip.x,
                clip_right,
                y,
                color,
                u8::MAX,
            );
            continue;
        }
        let left_edge = clip_right.min(left.saturating_add(radius));
        rendered += render_rounded_edge(
            framebuffer,
            width,
            height,
            stride,
            format,
            bounds,
            radius as u8,
            color,
            clip.x,
            left_edge,
            y,
        );
        rendered += fill_span(
            framebuffer,
            width,
            height,
            stride,
            format,
            clip.x.max(left.saturating_add(radius)),
            clip_right.min(right.saturating_sub(radius)),
            y,
            color,
            u8::MAX,
        );
        rendered += render_rounded_edge(
            framebuffer,
            width,
            height,
            stride,
            format,
            bounds,
            radius as u8,
            color,
            clip.x.max(right.saturating_sub(radius)),
            clip_right,
            y,
        );
    }
    rendered
}

#[allow(clippy::too_many_arguments)]
fn render_rounded_stroke(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    bounds: GuiRect,
    radius: u8,
    stroke_width: u8,
    color: u32,
    clip: GuiRect,
    inner: Option<(GuiRect, u8)>,
) -> usize {
    if clip.is_empty() {
        return 0;
    }
    let left = bounds.x;
    let right = bounds.x.saturating_add(bounds.width as i32);
    let top = bounds.y;
    let band = i32::from(radius).max(i32::from(stroke_width));
    let clip_right = clip.x.saturating_add(clip.width as i32);
    let mut rendered = 0;
    for y in clip.y..clip.y.saturating_add(clip.height as i32) {
        let local_y = y.saturating_sub(top);
        if local_y < band || local_y >= bounds.height as i32 - band {
            rendered += render_stroke_edge(
                framebuffer,
                width,
                height,
                stride,
                format,
                bounds,
                radius,
                color,
                inner,
                clip.x,
                clip_right,
                y,
            );
        } else {
            rendered += render_stroke_edge(
                framebuffer,
                width,
                height,
                stride,
                format,
                bounds,
                radius,
                color,
                inner,
                clip.x,
                clip_right.min(left.saturating_add(band)),
                y,
            );
            rendered += render_stroke_edge(
                framebuffer,
                width,
                height,
                stride,
                format,
                bounds,
                radius,
                color,
                inner,
                clip.x.max(right.saturating_sub(band)),
                clip_right,
                y,
            );
        }
    }
    rendered
}

#[allow(clippy::too_many_arguments)]
fn render_rounded_edge(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    bounds: GuiRect,
    radius: u8,
    color: u32,
    left: i32,
    right: i32,
    y: i32,
) -> usize {
    let mut rendered = 0;
    for x in left..right {
        let coverage = rounded_coverage(bounds, radius, x, y);
        if coverage != 0 {
            rendered +=
                plot(framebuffer, width, height, stride, format, x, y, color, coverage) as usize;
        }
    }
    rendered
}

#[allow(clippy::too_many_arguments)]
fn render_stroke_edge(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    bounds: GuiRect,
    radius: u8,
    color: u32,
    inner: Option<(GuiRect, u8)>,
    left: i32,
    right: i32,
    y: i32,
) -> usize {
    let mut rendered = 0;
    for x in left..right {
        let outer = rounded_coverage(bounds, radius, x, y);
        let inner =
            inner.map(|(bounds, radius)| rounded_coverage(bounds, radius, x, y)).unwrap_or(0);
        let coverage = outer.saturating_sub(inner);
        if coverage != 0 {
            rendered +=
                plot(framebuffer, width, height, stride, format, x, y, color, coverage) as usize;
        }
    }
    rendered
}

#[allow(clippy::too_many_arguments)]
fn render_line(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    command: GuiDrawCommand,
    clip: GuiRect,
) -> usize {
    let x0 = i64::from(command.x) * 4;
    let y0 = i64::from(command.y) * 4;
    let x1 = x0 + i64::from(command.width) * 4;
    let y1 = y0 + i64::from(command.height) * 4;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let length_squared = dx * dx + dy * dy;
    let radius = i64::from(command.line_width()) * 2;
    let radius_squared = radius * radius;
    let mut rendered = 0;
    for y in clip.y..clip.y.saturating_add(clip.height as i32) {
        for x in clip.x..clip.x.saturating_add(clip.width as i32) {
            let mut samples = 0;
            for sample_y in [1_i64, 3] {
                for sample_x in [1_i64, 3] {
                    if point_near_segment(
                        i64::from(x) * 4 + sample_x,
                        i64::from(y) * 4 + sample_y,
                        x0,
                        y0,
                        x1,
                        y1,
                        dx,
                        dy,
                        length_squared,
                        radius_squared,
                    ) {
                        samples += 1;
                    }
                }
            }
            let coverage = coverage_from_samples(samples);
            if coverage != 0 {
                rendered +=
                    plot(framebuffer, width, height, stride, format, x, y, command.color, coverage)
                        as usize;
            }
        }
    }
    rendered
}

#[allow(clippy::too_many_arguments)]
fn render_shadow(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    command: GuiDrawCommand,
    clip: GuiRect,
    shadow_shapes: &[GuiRect; 5],
    blur: u8,
) -> usize {
    let mut rendered = 0;
    for y in clip.y..clip.y.saturating_add(clip.height as i32) {
        for x in clip.x..clip.x.saturating_add(clip.width as i32) {
            let coverage = shadow_coverage(shadow_shapes, command.corner_radius(), x, y, blur);
            if coverage != 0 {
                rendered +=
                    plot(framebuffer, width, height, stride, format, x, y, command.color, coverage)
                        as usize;
            }
        }
    }
    rendered
}

#[allow(clippy::too_many_arguments)]
fn fill_span(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    left: i32,
    right: i32,
    y: i32,
    color: u32,
    coverage: u8,
) -> usize {
    let left = left.max(0).min(width as i32) as usize;
    let right = right.max(0).min(width as i32) as usize;
    if left >= right || y < 0 || y as usize >= height {
        return 0;
    }
    if coverage == u8::MAX && color_alpha(color) == u8::MAX {
        let pixel = super::pixel_bytes(color, format);
        let start = y as usize * stride + left * 4;
        let end = y as usize * stride + right * 4;
        super::fill_row(&mut framebuffer[start..end], pixel);
        return right - left;
    }
    let mut rendered = 0;
    for x in left..right {
        rendered +=
            plot(framebuffer, width, height, stride, format, x as i32, y, color, coverage) as usize;
    }
    rendered
}

fn rounded_stroke_inner(command: GuiDrawCommand, bounds: GuiRect) -> Option<(GuiRect, u8)> {
    let edge = u32::from(command.stroke_width());
    if edge.saturating_mul(2) < bounds.width.min(bounds.height) {
        Some((
            GuiRect::new(
                bounds.x.saturating_add(edge as i32),
                bounds.y.saturating_add(edge as i32),
                bounds.width.saturating_sub(edge.saturating_mul(2)),
                bounds.height.saturating_sub(edge.saturating_mul(2)),
            ),
            command.corner_radius().saturating_sub(command.stroke_width()),
        ))
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn shadow_coverage(shadow_shapes: &[GuiRect; 5], radius: u8, x: i32, y: i32, blur: u8) -> u8 {
    let weights = SHADOW_RING_WEIGHTS[blur as usize];
    let base = rounded_coverage(shadow_shapes[0], radius, x, y);
    if blur == 0 || base == u8::MAX {
        return base;
    }
    let mut weighted = u32::from(base) * u32::from(weights[0]);
    for distance in 1..=blur {
        let weight = u32::from(weights[distance as usize]);
        if weight == 0 {
            continue;
        }
        let mask = u32::from(rounded_coverage(
            shadow_shapes[distance as usize],
            radius.saturating_add(distance),
            x,
            y,
        ));
        weighted += mask * weight;
    }
    ((weighted + 127) / 255) as u8
}

const SHADOW_RING_WEIGHTS: [[u8; 5]; 5] = [
    [255, 0, 0, 0, 0],
    [160, 80, 15, 0, 0],
    [128, 80, 40, 7, 0],
    [96, 72, 56, 24, 7],
    [64, 64, 64, 48, 15],
];

#[allow(clippy::too_many_arguments)]
fn point_near_segment(
    px: i64,
    py: i64,
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    dx: i64,
    dy: i64,
    length_squared: i64,
    radius_squared: i64,
) -> bool {
    let projection = (px - x0) * dx + (py - y0) * dy;
    if projection <= 0 {
        let difference_x = px - x0;
        let difference_y = py - y0;
        difference_x * difference_x + difference_y * difference_y <= radius_squared
    } else if projection >= length_squared {
        let difference_x = px - x1;
        let difference_y = py - y1;
        difference_x * difference_x + difference_y * difference_y <= radius_squared
    } else {
        let cross = (px - x0) * dy - (py - y0) * dx;
        cross * cross <= radius_squared * length_squared
    }
}

fn rounded_coverage(bounds: GuiRect, radius: u8, x: i32, y: i32) -> u8 {
    if radius == 0 {
        return if bounds.contains(x, y) { u8::MAX } else { 0 };
    }
    let left = i64::from(bounds.x) * 4;
    let top = i64::from(bounds.y) * 4;
    let right = (i64::from(bounds.x) + i64::from(bounds.width)) * 4;
    let bottom = (i64::from(bounds.y) + i64::from(bounds.height)) * 4;
    let radius = i64::from(radius) * 4;
    let pixel_x = i64::from(x);
    let pixel_y = i64::from(y);
    let bounds_left = i64::from(bounds.x);
    let bounds_top = i64::from(bounds.y);
    let bounds_right = i64::from(bounds.x) + i64::from(bounds.width);
    let bounds_bottom = i64::from(bounds.y) + i64::from(bounds.height);
    if pixel_x < bounds_left
        || pixel_x >= bounds_right
        || pixel_y < bounds_top
        || pixel_y >= bounds_bottom
    {
        return 0;
    }
    if (pixel_x * 4 >= left + radius && pixel_x * 4 + 3 < right - radius)
        || (pixel_y * 4 >= top + radius && pixel_y * 4 + 3 < bottom - radius)
    {
        return u8::MAX;
    }
    let mut samples = 0;
    for sample_y in [1_i64, 3] {
        for sample_x in [1_i64, 3] {
            let px = i64::from(x) * 4 + sample_x;
            let py = i64::from(y) * 4 + sample_y;
            if rounded_sample_inside(left, top, right, bottom, radius, px, py) {
                samples += 1;
            }
        }
    }
    coverage_from_samples(samples)
}

fn rounded_sample_inside(
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
    radius: i64,
    px: i64,
    py: i64,
) -> bool {
    if px < left || py < top || px >= right || py >= bottom {
        return false;
    }
    if (px >= left + radius && px < right - radius) || (py >= top + radius && py < bottom - radius)
    {
        return true;
    }
    let center_x = if px < left + radius { left + radius } else { right - radius };
    let center_y = if py < top + radius { top + radius } else { bottom - radius };
    let dx = px - center_x;
    let dy = py - center_y;
    dx * dx + dy * dy <= radius * radius
}

fn coverage_from_samples(samples: u8) -> u8 {
    match samples {
        0 => 0,
        1 => 64,
        2 => 128,
        3 => 192,
        _ => u8::MAX,
    }
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
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
) -> bool {
    plot_packed(
        framebuffer,
        width,
        height,
        stride,
        format,
        x,
        y,
        color,
        super::pixel_bytes(color, format),
        coverage,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn plot_packed(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: super::PixelFormat,
    x: i32,
    y: i32,
    color: u32,
    packed: [u8; 4],
    coverage: u8,
) -> bool {
    if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
        return false;
    }
    let alpha = color_alpha(color);
    let coverage = ((u16::from(coverage) * u16::from(alpha) + 127) / 255) as u8;
    if coverage == 0 {
        return false;
    }
    let color = color & 0x00ff_ffff;
    let offset = y as usize * stride + x as usize * 4;
    if offset + 4 > framebuffer.len() {
        return false;
    }
    if coverage == u8::MAX {
        framebuffer[offset..offset + 4].copy_from_slice(&packed);
        return true;
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

fn color_alpha(color: u32) -> u8 {
    let alpha = (color >> 24) as u8;
    if alpha == 0 { u8::MAX } else { alpha }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PixelFormat;
    use logos_abi::GuiDrawCommand;
    use std::vec;

    struct CountingBackend {
        draws: usize,
    }

    impl GuiRenderBackend for CountingBackend {
        fn draw(&mut self, _command: GuiDrawCommand, _clip: GuiRect) -> usize {
            self.draws += 1;
            1
        }
    }

    struct RecordingBackend {
        commands: std::vec::Vec<GuiDrawCommand>,
    }

    impl GuiRenderBackend for RecordingBackend {
        fn draw(&mut self, command: GuiDrawCommand, _clip: GuiRect) -> usize {
            self.commands.push(command);
            1
        }
    }

    fn request(operation: GuiSurfaceOperation, id: u32, bounds: GuiRect) -> GuiSurfaceRequest {
        let mut request = GuiSurfaceRequest::new(operation, id);
        request.bounds = bounds;
        request.z_order = 1;
        request
    }

    fn render_single(
        command: GuiDrawCommand,
        damage: GuiRect,
        width: usize,
        height: usize,
        background: [u8; 4],
    ) -> std::vec::Vec<u8> {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(
                7,
                request(
                    GuiSurfaceOperation::CreateRoot,
                    1,
                    GuiRect::new(0, 0, width as u32, height as u32),
                ),
            )
            .unwrap()
            .surface;
        registry.take_damage();
        let mut batch = GuiDrawBatch::new(root, 1, damage);
        assert!(batch.push(command));
        registry.update(7, batch).unwrap();
        let (damage, damage_count) = registry.take_damage();
        let mut framebuffer = vec![0; width * height * 4];
        for pixel in framebuffer.chunks_exact_mut(4) {
            pixel.copy_from_slice(&background);
        }
        let mut glyph_cache = crate::GlyphCache::new();
        registry.render(
            &mut glyph_cache,
            &mut framebuffer,
            width,
            height,
            width * 4,
            PixelFormat::Bgr8,
            &damage,
            damage_count,
        );
        framebuffer
    }

    fn pixel(framebuffer: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * width + x) * 4;
        framebuffer[offset..offset + 4].try_into().unwrap()
    }

    #[test]
    fn rounded_fill_and_stroke_have_anti_aliased_corners() {
        let bounds = GuiRect::new(2, 2, 28, 20);
        let fill = render_single(
            GuiDrawCommand::fill_rounded_rect(bounds, 0xffffff, 8),
            bounds,
            32,
            24,
            [0; 4],
        );
        assert_eq!(pixel(&fill, 32, 2, 2), [0, 0, 0, 0]);
        assert_eq!(pixel(&fill, 32, 16, 12), [255, 255, 255, 0]);

        let stroke = render_single(
            GuiDrawCommand::stroke_rounded_rect(bounds, 0xffffff, 8, 2),
            bounds,
            32,
            24,
            [0; 4],
        );
        assert_eq!(pixel(&stroke, 32, 16, 2), [255, 255, 255, 0]);
        assert_eq!(pixel(&stroke, 32, 16, 12), [0, 0, 0, 0]);
    }

    #[test]
    fn thick_lines_and_alpha_shadows_blend_without_allocations() {
        let line = render_single(
            GuiDrawCommand::line_with_width(4, 4, 20, 0, 0xff0000, 3),
            GuiRect::new(0, 0, 32, 12),
            32,
            12,
            [0; 4],
        );
        assert_eq!(pixel(&line, 32, 12, 4), [0, 0, 255, 0]);
        assert_ne!(pixel(&line, 32, 12, 3), [0, 0, 0, 0]);

        let shadow = render_single(
            GuiDrawCommand::shadow(GuiRect::new(12, 8, 16, 8), 0x55000000, 4, 2, 0, 4),
            GuiRect::new(0, 0, 48, 32),
            48,
            32,
            [255; 4],
        );
        assert!(pixel(&shadow, 48, 20, 20)[0] < 255);
        assert_eq!(pixel(&shadow, 48, 2, 2), [255, 255, 255, 255]);
    }

    #[test]
    fn opaque_panel_occludes_shadow_work() {
        let mut framebuffer = vec![0xff; 48 * 32 * 4];
        let shadow = GuiDrawCommand::shadow(GuiRect::new(12, 8, 16, 8), 0x55000000, 4, 2, 0, 4);
        let mut clip = GuiRect::new(12, 8, 16, 8);
        let mut damage = [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS];
        damage[0] = clip;
        let mut occluders = [GuiRect::EMPTY; MAX_GUI_SURFACES * MAX_GUI_NODES];
        occluders[0] = clip;
        let mut glyph_cache = crate::GlyphCache::new();
        let mut backend = SoftwareRenderBackend {
            framebuffer: &mut framebuffer,
            glyph_cache: &mut glyph_cache,
            width: 48,
            height: 32,
            stride: 48 * 4,
            format: PixelFormat::Bgr8,
        };
        assert_eq!(render_one(&mut backend, shadow, &mut clip, &damage, 1, &occluders, 1,), 0);
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
    fn max_z_diagnostic_surface_renders_above_opaque_modals() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(11, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 64, 32)))
            .unwrap()
            .surface;
        let mut modal_request =
            request(GuiSurfaceOperation::CreateModal, 2, GuiRect::new(0, 0, 64, 32));
        let modal = registry.create(12, modal_request).unwrap().surface;
        modal_request.z_order = i16::MAX;
        let overlay = registry.create(11, modal_request).unwrap().surface;

        let mut root_batch = GuiDrawBatch::new(root, 1, GuiRect::SURFACE);
        assert!(root_batch.push(GuiDrawCommand::fill_surface(0x101820)));
        registry.update(11, root_batch).unwrap();
        let mut modal_batch = GuiDrawBatch::new(modal, 1, GuiRect::SURFACE);
        assert!(modal_batch.push(GuiDrawCommand::fill_surface(0x203040)));
        registry.update(12, modal_batch).unwrap();
        registry
            .apply_scene_op(
                11,
                GuiSceneOp::upsert(
                    overlay,
                    1,
                    99,
                    GuiDrawCommand::glyph_run(8, 8, 0xffffff, b"FPS:060").unwrap(),
                ),
            )
            .unwrap();

        let (damage, count) = registry.take_damage();
        let mut backend = RecordingBackend { commands: std::vec::Vec::new() };
        registry.compose(&mut backend, &damage, count);
        assert_eq!(backend.commands.last().unwrap().text_len, 7);
        assert_eq!(&backend.commands.last().unwrap().text[..7], b"FPS:060");
    }

    #[test]
    fn bounds_updates_damage_old_and_new_geometry() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 10, 10)))
            .unwrap()
            .surface;
        registry.take_damage();
        registry.set_bounds(7, root, GuiRect::new(10, 10, 20, 20)).unwrap();
        let (damage, count) = registry.take_damage();
        assert_eq!(count, 1);
        assert!(damage[0].contains(1, 1));
        assert!(damage[0].contains(20, 20));
    }

    #[test]
    fn no_op_surface_changes_do_not_add_damage() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 10, 10)))
            .unwrap()
            .surface;
        registry.take_damage();
        registry.set_bounds(7, root, GuiRect::new(0, 0, 10, 10)).unwrap();
        assert_eq!(registry.take_damage().1, 0);
        registry.focus(7, root).unwrap();
        registry.take_damage();
        registry.focus(7, root).unwrap();
        assert_eq!(registry.take_damage().1, 0);
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
    fn retained_scene_commits_are_atomic_and_damage_node_deltas() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 40, 20)))
            .unwrap()
            .surface;
        registry.take_damage();

        let mut first = GuiSceneOp::upsert(
            root,
            1,
            9,
            GuiDrawCommand::fill_rect(GuiRect::new(2, 2, 4, 4), 0xffffff),
        );
        first.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        registry.apply_scene_op(7, first).unwrap();
        assert_eq!(registry.active_frame(root), Some(0));
        assert_eq!(registry.take_damage().1, 0);

        first.flags = 0;
        registry.apply_scene_op(7, first).unwrap();
        assert_eq!(registry.active_frame(root), Some(1));
        let (damage, count) = registry.take_damage();
        assert_eq!(count, 1);
        assert!(damage[0].contains(2, 2));

        let mut moved = GuiSceneOp::upsert(
            root,
            2,
            9,
            GuiDrawCommand::fill_rect(GuiRect::new(24, 2, 4, 4), 0xffffff),
        );
        moved.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        registry.apply_scene_op(7, moved).unwrap();
        assert_eq!(registry.take_damage().1, 0);
        moved.flags = 0;
        registry.apply_scene_op(7, moved).unwrap();
        let (damage, count) = registry.take_damage();
        assert_eq!(count, 2);
        assert!(damage[..count].iter().any(|rect| rect.contains(2, 2)));
        assert!(damage[..count].iter().any(|rect| rect.contains(24, 2)));

        registry.apply_scene_op(7, GuiSceneOp::remove(root, 3, 9)).unwrap();
        let (damage, count) = registry.take_damage();
        assert_eq!(count, 1);
        assert!(damage[0].contains(24, 2));
    }

    #[test]
    fn retained_scene_keeps_static_nodes_across_dynamic_frames() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 40, 20)))
            .unwrap()
            .surface;
        registry.take_damage();

        let mut static_node = GuiSceneOp::upsert(
            root,
            1,
            1,
            GuiDrawCommand::glyph_run(2, 2, 0xffffff, b"Static").unwrap(),
        );
        static_node.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        registry.apply_scene_op(7, static_node).unwrap();
        static_node.flags = 0;
        registry.apply_scene_op(7, static_node).unwrap();
        registry.take_damage();

        let mut dynamic_node = GuiSceneOp::upsert(
            root,
            2,
            2,
            GuiDrawCommand::fill_rect(GuiRect::new(24, 18, 4, 2), 0xffffff),
        );
        dynamic_node.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        registry.apply_scene_op(7, dynamic_node).unwrap();
        assert_eq!(registry.active_frame(root), Some(1));
        dynamic_node.flags = 0;
        registry.apply_scene_op(7, dynamic_node).unwrap();
        assert_eq!(registry.active_frame(root), Some(2));

        registry.take_damage();
        let mut damage = [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS];
        damage[0] = GuiRect::new(0, 0, 40, 20);
        let mut backend = CountingBackend { draws: 0 };
        assert_eq!(registry.compose(&mut backend, &damage, 1), 2);
        assert_eq!(backend.draws, 2);
    }

    #[test]
    fn retained_scene_rejects_older_frames() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 16, 16)))
            .unwrap()
            .surface;
        registry.take_damage();
        let node = GuiSceneOp::upsert(
            root,
            2,
            1,
            GuiDrawCommand::fill_rect(GuiRect::new(2, 2, 4, 4), 0xffffff),
        );
        registry.apply_scene_op(7, node).unwrap();
        assert_eq!(registry.active_frame(root), Some(2));
        let stale = GuiSceneOp::upsert(
            root,
            1,
            1,
            GuiDrawCommand::fill_rect(GuiRect::new(8, 8, 4, 4), 0xffffff),
        );
        assert_eq!(registry.apply_scene_op(7, stale), Err(GuiRegistryError::Stale));
        assert_eq!(registry.active_frame(root), Some(2));
    }

    #[test]
    fn composition_is_backend_neutral() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 16, 16)))
            .unwrap()
            .surface;
        registry.take_damage();
        let mut batch = GuiDrawBatch::new(root, 1, GuiRect::new(2, 2, 4, 4));
        assert!(batch.push(GuiDrawCommand::fill_rect(GuiRect::new(2, 2, 4, 4), 0xffffff)));
        registry.update(7, batch).unwrap();
        let (damage, count) = registry.take_damage();
        let mut backend = CountingBackend { draws: 0 };
        assert_eq!(registry.compose(&mut backend, &damage, count), 1);
        assert_eq!(backend.draws, 1);
    }

    #[test]
    fn retained_composition_plan_is_reused_until_scene_changes() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 32, 32)))
            .unwrap()
            .surface;
        registry.take_damage();
        let mut batch = GuiDrawBatch::new(root, 1, GuiRect::new(2, 2, 8, 8));
        assert!(batch.push(GuiDrawCommand::fill_rect(GuiRect::new(2, 2, 8, 8), 0xffffff)));
        registry.update(7, batch).unwrap();
        let (damage, count) = registry.take_damage();
        let mut backend = CountingBackend { draws: 0 };

        registry.compose(&mut backend, &damage, count);
        assert!(registry.plan.valid);
        let entry_count = registry.plan.entry_count;
        registry.compose(&mut backend, &damage, count);
        assert_eq!(registry.plan.entry_count, entry_count);

        let mut replacement = GuiDrawBatch::new(root, 2, GuiRect::new(4, 4, 8, 8));
        assert!(replacement.push(GuiDrawCommand::fill_rect(GuiRect::new(4, 4, 8, 8), 0xffffff,)));
        registry.update(7, replacement).unwrap();
        assert!(!registry.plan.valid);
    }

    #[test]
    fn legacy_updates_damage_primitive_bounds_only() {
        let mut registry = GuiSurfaceRegistry::new();
        let root = registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 100, 100)))
            .unwrap()
            .surface;
        registry.take_damage();
        let bounds = GuiRect::new(2, 3, 4, 5);
        let mut batch = GuiDrawBatch::new(root, 1, bounds);
        assert!(batch.push(GuiDrawCommand::fill_rect(bounds, 0xffffff)));
        registry.update(7, batch).unwrap();
        let (damage, count) = registry.take_damage();
        assert_eq!(count, 1);
        assert_eq!(damage[0], bounds);
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

    #[test]
    fn terminal_marker_is_modal_and_singleton() {
        let mut registry = GuiSurfaceRegistry::new();
        registry
            .create(7, request(GuiSurfaceOperation::CreateRoot, 1, GuiRect::new(0, 0, 10, 10)))
            .unwrap();
        let mut terminal = request(GuiSurfaceOperation::CreateModal, 2, GuiRect::new(0, 0, 10, 10));
        terminal.flags = GUI_SURFACE_FLAG_TERMINAL;
        assert!(registry.create(7, terminal).is_ok());
        terminal.request_id = 3;
        assert_eq!(registry.create(7, terminal), Err(GuiRegistryError::Capacity));

        let mut root_terminal =
            request(GuiSurfaceOperation::CreateRoot, 4, GuiRect::new(0, 0, 10, 10));
        root_terminal.flags = GUI_SURFACE_FLAG_TERMINAL;
        let mut fresh = GuiSurfaceRegistry::new();
        assert_eq!(fresh.create(7, root_terminal), Err(GuiRegistryError::InvalidRequest));
    }
}
