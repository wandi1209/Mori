use std::collections::HashMap;

use anyhow::Result;
use crate::cursor::Cursor;

const TEMPORARY_ITEM_IDS: &[u16] = &[
    1424, // World Key
    5640, // Magplant 5000 Remote
];

#[derive(Debug, Clone)]
pub struct InventoryItem {
    pub id:     u16,
    pub amount: u8,
    pub flag:   u8,
}

#[derive(Debug, Default, Clone)]
pub struct Inventory {
    pub size:       u32,
    pub item_count: u16,
    pub items:      HashMap<u16, InventoryItem>,
    pub gems:       i32,
}

impl Inventory {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut cur = Cursor::new(data, "inventory");

        cur.skip(1)?;                        // unknown leading byte
        let size       = cur.u32()?;
        let item_count = cur.u16()?;

        let mut items = HashMap::with_capacity(item_count as usize);
        for _ in 0..item_count {
            let id     = cur.u16()?;
            let amount = cur.u8()?;
            let flag   = cur.u8()?;
            items.insert(id, InventoryItem { id, amount, flag });
        }

        Ok(Self { size, item_count, items, gems: 0 })
    }

    pub fn clear(&mut self) {
        self.size       = 0;
        self.item_count = 0;
        self.items.clear();
    }

    pub fn add_item(&mut self, id: u16, amount: u8) {
        if let Some(item) = self.items.get_mut(&id) {
            item.amount = item.amount.saturating_add(amount);
        } else {
            self.items.insert(id, InventoryItem { id, amount, flag: 0 });
            self.item_count += 1;
        }
    }

    pub fn add_gems(&mut self, amount: i32) {
        self.gems = amount;
    }

    pub fn has_item(&self, id: u16, min_amount: u8) -> bool {
        self.items.get(&id).map(|i| i.amount >= min_amount).unwrap_or(false)
    }

    pub fn can_collect(&self, item_id: u16) -> bool {
        if item_id == 112 {
            return true; // gems always fit
        }
        if let Some(existing) = self.items.get(&item_id) {
            existing.amount < 200
        } else {
            (self.item_count as u32) < self.size
        }
    }

    pub fn sub_item(&mut self, id: u16, amount: u8) {
        if let Some(item) = self.items.get_mut(&id) {
            if item.amount <= amount {
                self.items.remove(&id);
                self.item_count = self.item_count.saturating_sub(1);
            } else {
                item.amount -= amount;
            }
        }
    }

    pub fn remove_item(&mut self, id: u16) {
        if self.items.remove(&id).is_some() {
            self.item_count = self.item_count.saturating_sub(1);
        }
    }

    pub fn remove_temp_items(&mut self) -> bool {
        let to_remove: Vec<u16> = self.items.keys()
            .copied()
            .filter(|id| TEMPORARY_ITEM_IDS.contains(id))
            .collect();

        let changed = !to_remove.is_empty();
        for id in to_remove {
            self.remove_item(id);
        }

        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bag(size: u32, items: &[(u16, u8)]) -> Inventory {
        let mut inv = Inventory { size, ..Default::default() };
        for (id, amount) in items {
            inv.add_item(*id, *amount);
        }
        inv
    }

    #[test]
    fn a_stack_has_room_until_two_hundred() {
        let inv = bag(10, &[(2018, 199)]);
        assert!(inv.can_collect(2018));

        let inv = bag(10, &[(2018, 200)]);
        assert!(!inv.can_collect(2018));
    }

    #[test]
    fn a_new_item_needs_a_free_slot() {
        let inv = bag(2, &[(2018, 5)]);
        assert!(inv.can_collect(2019), "one slot is still free");

        let inv = bag(1, &[(2018, 5)]);
        assert!(!inv.can_collect(2019), "no slot left for a new item");
    }

    #[test]
    fn gems_fit_even_when_every_slot_is_taken() {
        // Gems are currency, not an inventory item — a full bag does not stop them.
        let inv = bag(1, &[(2018, 200)]);
        assert!(inv.can_collect(112));
    }
}
