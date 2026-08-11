// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II 精灵寄存器状态
//
//   文件:       sprite.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

const ENABLED: u8 = 1 << 0;
const FOREGROUND: u8 = 1 << 1;
const MULTICOLOR: u8 = 1 << 2;
const EXPAND_VERTICAL: u8 = 1 << 3;
const EXPAND_HORIZONTAL: u8 = 1 << 4;
const COLLISION_WITH_SPRITE: u8 = 1 << 5;
const COLLISION_WITH_FOREGROUND: u8 = 1 << 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicSprite {
    pub x: u16,
    pub y: u8,
    pub color: u32,
    flags: u8,
}

impl VicSprite {
    pub const fn new(color: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            color,
            flags: FOREGROUND,
        }
    }

    pub fn reset(&mut self, color: u32) {
        *self = Self::new(color);
    }

    pub const fn enabled(self) -> bool {
        self.flag(ENABLED)
    }

    pub const fn foreground(self) -> bool {
        self.flag(FOREGROUND)
    }

    pub const fn multicolor(self) -> bool {
        self.flag(MULTICOLOR)
    }

    pub const fn expand_vertical(self) -> bool {
        self.flag(EXPAND_VERTICAL)
    }

    pub const fn expand_horizontal(self) -> bool {
        self.flag(EXPAND_HORIZONTAL)
    }

    pub const fn collision_with_sprite(self) -> bool {
        self.flag(COLLISION_WITH_SPRITE)
    }

    pub const fn collision_with_foreground(self) -> bool {
        self.flag(COLLISION_WITH_FOREGROUND)
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.set_flag(ENABLED, enabled);
    }

    pub fn set_foreground(&mut self, foreground: bool) {
        self.set_flag(FOREGROUND, foreground);
    }

    pub fn set_multicolor(&mut self, multicolor: bool) {
        self.set_flag(MULTICOLOR, multicolor);
    }

    pub fn set_expand_vertical(&mut self, expanded: bool) {
        self.set_flag(EXPAND_VERTICAL, expanded);
    }

    pub fn set_expand_horizontal(&mut self, expanded: bool) {
        self.set_flag(EXPAND_HORIZONTAL, expanded);
    }

    pub fn set_collision_with_sprite(&mut self, collided: bool) {
        self.set_flag(COLLISION_WITH_SPRITE, collided);
    }

    pub fn set_collision_with_foreground(&mut self, collided: bool) {
        self.set_flag(COLLISION_WITH_FOREGROUND, collided);
    }

    const fn flag(self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    fn set_flag(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::VicSprite;

    #[test]
    fn reset_restores_only_the_architectural_foreground_default() {
        let mut sprite = VicSprite::new(0x1122_3344);
        sprite.x = 0x1ff;
        sprite.set_enabled(true);
        sprite.set_multicolor(true);
        sprite.set_collision_with_sprite(true);
        sprite.reset(0x5566_7788);
        assert_eq!(sprite.x, 0);
        assert_eq!(sprite.color, 0x5566_7788);
        assert!(sprite.foreground());
        assert!(!sprite.enabled());
        assert!(!sprite.multicolor());
        assert!(!sprite.collision_with_sprite());
    }
}
