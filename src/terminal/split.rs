//! Client-local sizing of a composition's two-column split.
use super::{modals, Rect};
use crate::{core::Core, sidebar::Declarations};
use trailboss_runtime::protocol::widgets::Node;

#[derive(Default)]
pub struct SidebarResize {
    desired_width: Option<u16>,
    captured: bool,
}

impl SidebarResize {
    /// Capture only the first pane column, leaving sidebar controls untouched.
    pub fn begin(
        &mut self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        x: u16,
        y: u16,
    ) -> bool {
        if !Self::on_boundary(core, declarations, viewport, x, y) {
            return false;
        }
        self.captured = true;
        true
    }

    /// Keep the resize pointer during capture, including beyond the viewport.
    pub fn resize_cursor(
        &self,
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        pointer: Option<(u16, u16)>,
    ) -> bool {
        self.captured
            || pointer.is_some_and(|(x, y)| Self::on_boundary(core, declarations, viewport, x, y))
    }

    fn on_boundary(
        core: &Core,
        declarations: &Declarations,
        viewport: Rect,
        x: u16,
        y: u16,
    ) -> bool {
        if viewport.width < 2
            || x < viewport.x
            || x - viewport.x >= viewport.width
            || y < viewport.y
            || y - viewport.y >= viewport.height
            || modals::dialog(core, declarations, viewport).is_some()
        {
            return false;
        }
        let Some((_, root)) = core.composition(declarations) else {
            return false;
        };
        let Some((left, right)) = weights(core, root) else {
            return false;
        };
        let sum = u32::from(left) + u32::from(right);
        sum != 0 && u32::from(x - viewport.x) == u32::from(viewport.width) * u32::from(left) / sum
    }

    /// Change only the winning composition's existing weights, without cloning trees.
    /// Viewport clamps do not replace the user's desired width.
    pub fn apply(&self, core: &Core, declarations: &mut Declarations, viewport: Rect) -> bool {
        let Some(desired) = self.desired_width.filter(|_| viewport.width >= 2) else {
            return false;
        };
        let Some((_, root)) = core.composition(declarations) else {
            return false;
        };
        if weights(core, root).is_none() {
            return false;
        }
        let Some(index) = declarations.iter().position(|(_, declaration)| {
            declaration
                .composition
                .as_ref()
                .is_some_and(|composition| std::ptr::eq(&composition.tree, root))
        }) else {
            return false;
        };
        let Node::Row { children, .. } = &mut declarations[index]
            .1
            .composition
            .as_mut()
            .expect("selected composition")
            .tree
        else {
            return false;
        };
        let width = clamp(desired, viewport.width);
        let mut changed = false;
        for (node, requested) in children.iter_mut().zip([width, viewport.width - width]) {
            if let Node::Flex { weight, .. } = node {
                changed |= *weight != requested;
                *weight = requested;
            }
        }
        changed
    }

    /// Captured movement is consumed even when it leaves the split unchanged.
    pub fn drag_to(&mut self, column: u16, viewport: Rect) -> bool {
        if !self.captured {
            return false;
        }
        if viewport.width >= 2 {
            self.desired_width = Some(clamp(column.saturating_sub(viewport.x), viewport.width));
        }
        true
    }

    pub fn captures_input(&self) -> bool {
        self.captured
    }

    pub fn finish(&mut self) -> bool {
        std::mem::take(&mut self.captured)
    }
}

fn clamp(width: u16, viewport_width: u16) -> u16 {
    let minimum = 12.min(viewport_width / 2);
    width.clamp(minimum, viewport_width - minimum)
}

fn weights(core: &Core, root: &Node) -> Option<(u16, u16)> {
    let Node::Row { children, .. } = root else {
        return None;
    };
    let [Node::Flex {
        weight: left,
        child: left_child,
    }, Node::Flex {
        weight: right,
        child: right_child,
    }] = children.as_slice()
    else {
        return None;
    };
    // Hidden branches have no allocation in the application walker.
    core.resolve(left_child)?;
    core.resolve(right_child)?;
    Some((*left, *right))
}
