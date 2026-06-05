#[rustfmt::skip]
pub mod pb;

pub mod constants;

use crate::protocol::constants::entity;
use crate::protocol::pb::EEntityType;

impl From<i64> for EEntityType {
    fn from(entity_type: i64) -> Self {
        match entity_type & entity::TYPE_MASK as i64 {
            64 => EEntityType::EntMonster,
            640 => EEntityType::EntChar,
            _ => EEntityType::EntErrType,
        }
    }
}
