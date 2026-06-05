pub mod entity {
    pub const TYPE_MASK: u16 = 0xFFFF;

    #[inline]
    pub fn get_player_uid(uuid: i64) -> i64 {
        uuid >> 16
    }
}

pub mod attr_type {
    pub const ATTR_NAME: i32 = 0x01;
    pub const ATTR_ID: i32 = 0x0A;
    pub const ATTR_PROFESSION_ID: i32 = 0xDC;
    pub const ATTR_FIGHT_POINT: i32 = 0x272E;
    pub const ATTR_ELITE_STATUS: i32 = 0xB6; // Elite/boss status flag
}

pub mod damage {
    pub const CRIT_BIT: i32 = 0b00000001;
}
