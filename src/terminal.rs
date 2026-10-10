use crate::Surface;
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use std::io::{self, Write};
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}
#[derive(Debug)]
pub struct Layout {
    pub working: Rect,
    pub list: Rect,
    pub footer: Rect,
    pub footer_right: Rect,
}
impl Layout {
    pub fn new(cols: u16, rows: u16) -> io::Result<Self> {
        if cols < 4 || rows < 2 {
            return Err(io::Error::other(
                "terminal must be at least 4 columns by 2 rows",
            ));
        }
        let sidebar = 20.min(cols / 4);
        let right = cols / 2;
        Ok(Self {
            working: Rect {
                x: sidebar,
                y: 0,
                width: cols - sidebar,
                height: rows - 1,
            },
            list: Rect {
                x: 0,
                y: 0,
                width: sidebar,
                height: rows - 1,
            },
            footer: Rect {
                x: 0,
                y: rows - 1,
                width: cols - right,
                height: 1,
            },
            footer_right: Rect {
                x: cols - right,
                y: rows - 1,
                width: right,
                height: 1,
            },
        })
    }
}
fn color(v: u32) -> Color {
    match v >> 24 {
        1 => Color::AnsiValue(v as u8),
        2 => Color::Rgb {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        },
        _ => match v {
            0 => Color::Reset,
            n => Color::AnsiValue((n - 1) as u8),
        },
    }
}
/// Draw only the PTY Surface. Reserved Zones remain empty until a plugin exists.
pub fn draw(out: &mut impl Write, surface: &Surface, layout: &Layout) -> io::Result<()> {
    queue!(
        out,
        cursor::Hide,
        SetAttribute(Attribute::Reset),
        SetBackgroundColor(Color::Reset),
        Clear(ClearType::All)
    )?;
    for y in 0..surface.height.min(layout.working.height) {
        for x in 0..surface.width.min(layout.working.width) {
            let c = &surface.cells[usize::from(y) * usize::from(surface.width) + usize::from(x)];
            queue!(
                out,
                cursor::MoveTo(layout.working.x + x, layout.working.y + y),
                SetAttribute(Attribute::Reset),
                SetForegroundColor(color(c.foreground)),
                SetBackgroundColor(color(c.background))
            )?;
            if c.modifiers & 1 != 0 {
                queue!(out, SetAttribute(Attribute::Bold))?;
            }
            if c.modifiers & 4 != 0 {
                queue!(out, SetAttribute(Attribute::Italic))?;
            }
            if c.modifiers & 8 != 0 {
                queue!(out, SetAttribute(Attribute::Underlined))?;
            }
            if c.modifiers & 2 != 0 {
                queue!(out, SetAttribute(Attribute::Dim))?;
            }
            if c.modifiers & 16 != 0 {
                queue!(out, SetAttribute(Attribute::SlowBlink))?;
            }
            if c.modifiers & 256 != 0 {
                queue!(out, SetAttribute(Attribute::CrossedOut))?;
            }
            queue!(out, Print(&c.symbol))?;
        }
    }
    if let Some((x, y)) = surface.cursor {
        queue!(
            out,
            cursor::MoveTo(layout.working.x + x, layout.working.y + y),
            cursor::Show
        )?;
    }
    out.flush()
}
