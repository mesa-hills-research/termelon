//! The items in order, their heights and the prefix sums over them, and the table from an
//! item's id to its place.
//!
//! Items sit in leaves of up to [`LEAF`] in order, and three Fenwick trees over the leaves keep
//! the sum of their heights, their count, and whether each holds a stale height. A lookup by id,
//! by offset or by rank, and a change of one height, each cost a walk of a tree and a scan of one
//! leaf. A leaf that splits or empties rebuilds the trees, which costs the number of leaves.
//!
//! A height is fresh when it was measured at the current epoch. Bumping the epoch makes every
//! height stale without visiting any item, only the leaves.
//!
//! An id names a slot, which holds the item's leaf while it lives, and a removed item's slot is
//! reused under a new generation. Slots and leaves are kept in chunks that never move, so adding
//! an item never stops to copy or rehash a table.

use core::cmp::Ordering;
use core::ops::{Index as At, IndexMut};

use super::ItemId;

/// The most items in a leaf.
const LEAF: usize = 64;

/// The slots in each chunk of the slot table.
const SLOT_CHUNK: usize = 1024;

/// The leaves in each chunk of the leaves' table.
const LEAF_CHUNK: usize = 64;

/// No slot, at the end of the list of free ones.
const NONE: u32 = u32::MAX;

/// A count or a place as a `u32`, which the index's sizes fit in.
fn small(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A list that grows a chunk of `N` at a time, so what it holds never moves once added.
#[derive(Debug)]
struct Chunked<T, const N: usize> {
    chunks: Vec<Vec<T>>,
    len: usize,
}

impl<T, const N: usize> Chunked<T, N> {
    fn new() -> Self {
        Self {
            chunks: Vec::new(),
            len: 0,
        }
    }

    fn get(&self, i: usize) -> Option<&T> {
        self.chunks.get(i / N)?.get(i % N)
    }

    fn get_mut(&mut self, i: usize) -> Option<&mut T> {
        self.chunks.get_mut(i / N)?.get_mut(i % N)
    }

    /// Adds `value` at the end and gives its place.
    fn push(&mut self, value: T) -> usize {
        if self.len.is_multiple_of(N) {
            self.chunks.push(Vec::with_capacity(N));
        }
        if let Some(chunk) = self.chunks.last_mut() {
            chunk.push(value);
        }
        self.len += 1;
        self.len - 1
    }

    /// Two of its values at once, at the places `a` and `b`, which differ.
    fn two_mut(&mut self, a: usize, b: usize) -> (&mut T, &mut T) {
        let (ca, cb) = (a / N, b / N);
        match ca.cmp(&cb) {
            Ordering::Equal => {
                let chunk = &mut self.chunks[ca];
                let (a, b) = (a % N, b % N);
                if a < b {
                    let (left, right) = chunk.split_at_mut(b);
                    (&mut left[a], &mut right[0])
                } else {
                    let (left, right) = chunk.split_at_mut(a);
                    (&mut right[0], &mut left[b])
                }
            }
            Ordering::Less => {
                let (left, right) = self.chunks.split_at_mut(cb);
                (&mut left[ca][a % N], &mut right[0][b % N])
            }
            Ordering::Greater => {
                let (left, right) = self.chunks.split_at_mut(ca);
                (&mut right[0][a % N], &mut left[cb][b % N])
            }
        }
    }
}

impl<T, const N: usize> At<usize> for Chunked<T, N> {
    type Output = T;

    fn index(&self, i: usize) -> &T {
        &self.chunks[i / N][i % N]
    }
}

impl<T, const N: usize> IndexMut<usize> for Chunked<T, N> {
    fn index_mut(&mut self, i: usize) -> &mut T {
        &mut self.chunks[i / N][i % N]
    }
}

/// An item's place, a leaf and an index in it. It holds until the next insert or remove.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cur {
    leaf: u32,
    at: u32,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    /// The leaf that holds the item while the slot is taken, and the next free slot while it
    /// isn't.
    leaf: u32,
    /// Odd while the slot is taken. It goes up by one as it's taken and as it's freed, so the id
    /// of an item that was removed finds nothing.
    generation: u32,
}

/// The table from an item's id to its leaf.
#[derive(Debug)]
struct Slots {
    slots: Chunked<Slot, SLOT_CHUNK>,
    /// The first free slot, or [`NONE`].
    free: u32,
}

impl Slots {
    fn new() -> Self {
        Self {
            slots: Chunked::new(),
            free: NONE,
        }
    }

    fn slot(&self, slot: u32) -> Option<&Slot> {
        self.slots.get(slot as usize)
    }

    fn slot_mut(&mut self, slot: u32) -> Option<&mut Slot> {
        self.slots.get_mut(slot as usize)
    }

    /// The leaf of the item `id` names, if it's still in the index.
    fn leaf(&self, id: ItemId) -> Option<u32> {
        self.slot(id.slot)
            .filter(|slot| slot.generation == id.generation)
            .map(|slot| slot.leaf)
    }

    /// The id of the item in `slot`.
    fn id(&self, slot: u32) -> ItemId {
        ItemId {
            slot,
            generation: self.slot(slot).map_or(0, |slot| slot.generation),
        }
    }

    /// Takes a slot for an item in `leaf`.
    fn take(&mut self, leaf: u32) -> ItemId {
        let slot = self.free;
        if let Some(entry) = self.slot_mut(slot) {
            let next = entry.leaf;
            entry.leaf = leaf;
            entry.generation = entry.generation.wrapping_add(1);
            let generation = entry.generation;
            self.free = next;
            return ItemId { slot, generation };
        }
        let slot = self.slots.push(Slot {
            leaf,
            generation: 1,
        });
        ItemId {
            slot: small(slot),
            generation: 1,
        }
    }

    fn set_leaf(&mut self, slot: u32, leaf: u32) {
        if let Some(entry) = self.slot_mut(slot) {
            entry.leaf = leaf;
        }
    }

    fn free(&mut self, slot: u32) {
        let next = self.free;
        if let Some(entry) = self.slot_mut(slot) {
            entry.generation = entry.generation.wrapping_add(1);
            entry.leaf = next;
            self.free = slot;
        }
    }
}

#[derive(Debug)]
struct Leaf<K> {
    keys: Vec<K>,
    /// Each item's slot.
    slots: Vec<u32>,
    heights: Vec<u32>,
    /// The epoch each height was measured at, or 0 for a hint.
    epochs: Vec<u32>,
    sum: u64,
    /// How many of its heights were measured at `fresh_epoch`.
    fresh: u32,
    fresh_epoch: u32,
    /// Whether it's counted among the leaves that hold a stale height.
    stale: bool,
    /// Its place among the leaves.
    pos: u32,
}

impl<K> Leaf<K> {
    fn new(pos: u32) -> Self {
        Self {
            keys: Vec::with_capacity(LEAF),
            slots: Vec::with_capacity(LEAF),
            heights: Vec::with_capacity(LEAF),
            epochs: Vec::with_capacity(LEAF),
            sum: 0,
            fresh: 0,
            fresh_epoch: 0,
            stale: false,
            pos,
        }
    }

    fn len(&self) -> usize {
        self.keys.len()
    }

    fn fresh_at(&self, epoch: u32) -> u32 {
        if self.fresh_epoch == epoch {
            self.fresh
        } else {
            0
        }
    }
}

/// Sums over the leaves, by their place.
#[derive(Debug, Default)]
struct Fenwick {
    /// One-based, as Fenwick trees are.
    tree: Vec<u64>,
}

impl Fenwick {
    fn rebuild(&mut self, values: impl Iterator<Item = u64>) {
        self.tree.clear();
        self.tree.push(0);
        self.tree.extend(values);
        for i in 1..self.tree.len() {
            let parent = i + (i & i.wrapping_neg());
            if parent < self.tree.len() {
                self.tree[parent] = self.tree[parent].wrapping_add(self.tree[i]);
            }
        }
    }

    fn push(&mut self, value: u64) {
        if self.tree.is_empty() {
            self.tree.push(0);
        }
        let i = self.tree.len();
        let low = i & i.wrapping_neg();
        let below = self.prefix(i - 1).wrapping_sub(self.prefix(i - low));
        self.tree.push(below.wrapping_add(value));
    }

    fn add(&mut self, at: usize, delta: i64) {
        let mut i = at + 1;
        while i < self.tree.len() {
            self.tree[i] = self.tree[i].wrapping_add_signed(delta);
            i += i & i.wrapping_neg();
        }
    }

    /// The sum of the first `count` values.
    fn prefix(&self, count: usize) -> u64 {
        let mut i = count.min(self.tree.len().saturating_sub(1));
        let mut sum = 0u64;
        while i > 0 {
            sum = sum.wrapping_add(self.tree[i]);
            i -= i & i.wrapping_neg();
        }
        sum
    }

    fn total(&self) -> u64 {
        self.prefix(self.tree.len().saturating_sub(1))
    }

    /// The most values whose sum is at most `target`, and that sum.
    fn search(&self, target: u64) -> (usize, u64) {
        let n = self.tree.len().saturating_sub(1);
        let mut step = if n == 0 { 0 } else { 1 << n.ilog2() };
        let (mut pos, mut sum) = (0, 0u64);
        while step > 0 {
            let next = pos + step;
            if next <= n && sum + self.tree[next] <= target {
                pos = next;
                sum += self.tree[next];
            }
            step >>= 1;
        }
        (pos, sum)
    }
}

#[derive(Debug)]
pub(crate) struct Index<K> {
    /// The leaves, by id. A freed one is empty and listed in `free`.
    slab: Chunked<Leaf<K>, LEAF_CHUNK>,
    free: Vec<u32>,
    /// The leaves' ids in order.
    order: Vec<u32>,
    sums: Fenwick,
    counts: Fenwick,
    /// One for each leaf that holds a stale height.
    stale: Fenwick,
    slots: Slots,
    len: usize,
    epoch: u32,
    fresh: usize,
}

impl<K> Index<K> {
    pub(crate) fn new() -> Self {
        Self {
            slab: Chunked::new(),
            free: Vec::new(),
            order: Vec::new(),
            sums: Fenwick::default(),
            counts: Fenwick::default(),
            stale: Fenwick::default(),
            slots: Slots::new(),
            len: 0,
            epoch: 1,
            fresh: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn total(&self) -> u64 {
        self.sums.total()
    }

    pub(crate) fn all_fresh(&self) -> bool {
        self.fresh == self.len
    }

    /// Makes every height stale. This costs the number of leaves.
    pub(crate) fn bump_epoch(&mut self) {
        // (Epoch 0 marks a hint, so it's skipped when the count wraps.)
        self.epoch = self.epoch.wrapping_add(1).max(1);
        self.fresh = 0;
        for &id in &self.order {
            self.slab[id as usize].stale = true;
        }
        self.stale.rebuild(self.order.iter().map(|_| 1));
    }

    /// Counts the leaf among the stale ones, or not, as its heights say.
    fn count_stale(&mut self, id: u32) {
        let leaf = &mut self.slab[id as usize];
        let stale = leaf.fresh_at(self.epoch) as usize != leaf.len();
        if stale != leaf.stale {
            leaf.stale = stale;
            let pos = leaf.pos as usize;
            self.stale.add(pos, if stale { 1 } else { -1 });
        }
    }

    pub(crate) fn key(&self, cur: Cur) -> &K {
        &self.slab[cur.leaf as usize].keys[cur.at as usize]
    }

    pub(crate) fn id(&self, cur: Cur) -> ItemId {
        self.slots
            .id(self.slab[cur.leaf as usize].slots[cur.at as usize])
    }

    pub(crate) fn height(&self, cur: Cur) -> u32 {
        self.slab[cur.leaf as usize].heights[cur.at as usize]
    }

    pub(crate) fn is_fresh(&self, cur: Cur) -> bool {
        self.slab[cur.leaf as usize].epochs[cur.at as usize] == self.epoch
    }

    pub(crate) fn locate(&self, id: ItemId) -> Option<Cur> {
        let leaf = self.slots.leaf(id)?;
        let at = self
            .slab
            .get(leaf as usize)?
            .slots
            .iter()
            .position(|&slot| slot == id.slot)?;
        Some(Cur {
            leaf,
            at: small(at),
        })
    }

    pub(crate) fn first(&self) -> Option<Cur> {
        self.order.first().map(|&leaf| Cur { leaf, at: 0 })
    }

    pub(crate) fn last(&self) -> Option<Cur> {
        self.order.last().map(|&leaf| Cur {
            leaf,
            at: small(self.slab[leaf as usize].len().saturating_sub(1)),
        })
    }

    pub(crate) fn next(&self, cur: Cur) -> Option<Cur> {
        let leaf = &self.slab[cur.leaf as usize];
        if (cur.at as usize) + 1 < leaf.len() {
            return Some(Cur {
                leaf: cur.leaf,
                at: cur.at + 1,
            });
        }
        let next = *self.order.get(leaf.pos as usize + 1)?;
        Some(Cur { leaf: next, at: 0 })
    }

    pub(crate) fn prev(&self, cur: Cur) -> Option<Cur> {
        if cur.at > 0 {
            return Some(Cur {
                leaf: cur.leaf,
                at: cur.at - 1,
            });
        }
        let pos = self.slab[cur.leaf as usize].pos as usize;
        let prev = *self.order.get(pos.checked_sub(1)?)?;
        Some(Cur {
            leaf: prev,
            at: small(self.slab[prev as usize].len().saturating_sub(1)),
        })
    }

    /// How many items come before it.
    pub(crate) fn rank(&self, cur: Cur) -> usize {
        let pos = self.slab[cur.leaf as usize].pos as usize;
        usize::try_from(self.counts.prefix(pos)).unwrap_or(usize::MAX) + cur.at as usize
    }

    /// The height of the items before it.
    pub(crate) fn offset_of(&self, cur: Cur) -> u64 {
        let leaf = &self.slab[cur.leaf as usize];
        let within: u64 = leaf.heights[..cur.at as usize]
            .iter()
            .map(|h| u64::from(*h))
            .sum();
        self.sums.prefix(leaf.pos as usize) + within
    }

    /// The item that covers the distance `offset` from the start, and how far into it that is.
    /// Past the feed's end, the last item, at its last row.
    pub(crate) fn find(&self, offset: u64) -> Option<(Cur, u32)> {
        let (pos, before) = self.sums.search(offset);
        let Some(&leaf) = self.order.get(pos) else {
            let last = self.last()?;
            return Some((last, self.height(last).saturating_sub(1)));
        };
        let mut rest = offset - before;
        for (at, height) in self.slab[leaf as usize].heights.iter().enumerate() {
            let height = u64::from(*height);
            if rest < height {
                return Some((
                    Cur {
                        leaf,
                        at: small(at),
                    },
                    u32::try_from(rest).unwrap_or(u32::MAX),
                ));
            }
            rest -= height;
        }
        // (A leaf's sum is its heights', so the search can't end past one.)
        None
    }

    /// Records the item's exact height.
    pub(crate) fn set(&mut self, cur: Cur, height: u32) {
        let epoch = self.epoch;
        let leaf = &mut self.slab[cur.leaf as usize];
        let at = cur.at as usize;
        let delta = i64::from(height) - i64::from(leaf.heights[at]);
        leaf.heights[at] = height;
        leaf.sum = leaf.sum.wrapping_add_signed(delta);
        if leaf.epochs[at] != epoch {
            leaf.epochs[at] = epoch;
            if leaf.fresh_epoch != epoch {
                leaf.fresh_epoch = epoch;
                leaf.fresh = 0;
            }
            leaf.fresh += 1;
            self.fresh += 1;
        }
        if delta != 0 {
            let pos = leaf.pos as usize;
            self.sums.add(pos, delta);
        }
        self.count_stale(cur.leaf);
    }

    /// Counts the item at `hint` until it's measured, unless it's measured already.
    pub(crate) fn set_hint(&mut self, cur: Cur, hint: u32) {
        if self.is_fresh(cur) {
            return;
        }
        let leaf = &mut self.slab[cur.leaf as usize];
        let at = cur.at as usize;
        let delta = i64::from(hint) - i64::from(leaf.heights[at]);
        leaf.heights[at] = hint;
        leaf.sum = leaf.sum.wrapping_add_signed(delta);
        let pos = leaf.pos as usize;
        self.sums.add(pos, delta);
    }

    /// Makes the item's height a hint again.
    pub(crate) fn invalidate(&mut self, cur: Cur) {
        let epoch = self.epoch;
        let leaf = &mut self.slab[cur.leaf as usize];
        let at = cur.at as usize;
        if leaf.epochs[at] == epoch {
            leaf.epochs[at] = 0;
            leaf.fresh -= 1;
            self.fresh -= 1;
        }
        self.count_stale(cur.leaf);
    }

    pub(crate) fn push(&mut self, key: K, hint: u32) -> ItemId {
        let room = self
            .order
            .last()
            .is_some_and(|&leaf| self.slab[leaf as usize].len() < LEAF);
        if !room {
            let id = self.new_leaf(small(self.order.len()));
            self.order.push(id);
            self.sums.push(0);
            self.counts.push(0);
            self.stale.push(0);
        }
        let leaf_id = self.order[self.order.len() - 1];
        let item = self.slots.take(leaf_id);
        let leaf = &mut self.slab[leaf_id as usize];
        leaf.keys.push(key);
        leaf.slots.push(item.slot);
        leaf.heights.push(hint);
        leaf.epochs.push(0);
        leaf.sum += u64::from(hint);
        let pos = leaf.pos as usize;
        self.sums.add(pos, i64::from(hint));
        self.counts.add(pos, 1);
        self.len += 1;
        self.count_stale(leaf_id);
        item
    }

    /// Adds an item before the item at `cur`.
    pub(crate) fn insert(&mut self, cur: Cur, key: K, hint: u32) -> ItemId {
        let id = cur.leaf;
        let at = cur.at as usize;
        let item = self.slots.take(id);
        let leaf = &mut self.slab[id as usize];
        leaf.keys.insert(at, key);
        leaf.slots.insert(at, item.slot);
        leaf.heights.insert(at, hint);
        leaf.epochs.insert(at, 0);
        leaf.sum += u64::from(hint);
        let pos = leaf.pos as usize;
        self.len += 1;
        if self.slab[id as usize].len() <= LEAF {
            self.sums.add(pos, i64::from(hint));
            self.counts.add(pos, 1);
            self.count_stale(id);
            return item;
        }
        // The leaf is full, so its second half moves to a new one after it.
        let new = self.new_leaf(small(pos + 1));
        let epoch = self.epoch;
        let (old, moved) = self.slab.two_mut(id as usize, new as usize);
        let half = old.len() / 2;
        moved.keys.extend(old.keys.drain(half..));
        moved.slots.extend(old.slots.drain(half..));
        moved.heights.extend(old.heights.drain(half..));
        moved.epochs.extend(old.epochs.drain(half..));
        for leaf in [&mut *old, &mut *moved] {
            leaf.sum = leaf.heights.iter().map(|h| u64::from(*h)).sum();
            leaf.fresh = small(leaf.epochs.iter().filter(|e| **e == epoch).count());
            leaf.fresh_epoch = epoch;
            leaf.stale = leaf.fresh as usize != leaf.len();
        }
        for i in 0..self.slab[new as usize].slots.len() {
            let slot = self.slab[new as usize].slots[i];
            self.slots.set_leaf(slot, new);
        }
        self.order.insert(pos + 1, new);
        self.reorder(pos + 1);
        item
    }

    pub(crate) fn remove(&mut self, cur: Cur) -> K {
        let epoch = self.epoch;
        let leaf = &mut self.slab[cur.leaf as usize];
        let at = cur.at as usize;
        let key = leaf.keys.remove(at);
        let slot = leaf.slots.remove(at);
        let height = leaf.heights.remove(at);
        if leaf.epochs.remove(at) == epoch {
            leaf.fresh -= 1;
            self.fresh -= 1;
        }
        leaf.sum -= u64::from(height);
        let pos = leaf.pos as usize;
        let empty = leaf.keys.is_empty();
        self.slots.free(slot);
        self.len -= 1;
        if empty {
            self.order.remove(pos);
            self.free.push(cur.leaf);
            self.reorder(pos);
        } else {
            self.sums.add(pos, -i64::from(height));
            self.counts.add(pos, -1);
            self.count_stale(cur.leaf);
        }
        key
    }

    /// An item that isn't fresh, in the nearest leaf to `near` that holds one, looking both
    /// ways.
    pub(crate) fn next_stale(&self, near: Cur) -> Option<Cur> {
        let pos = self.slab[near.leaf as usize].pos as usize;
        let before = self.stale.prefix(pos);
        // The first stale leaf at or after it, and the last one before it.
        let (below, _) = self.stale.search(before);
        let above = before.checked_sub(1).map(|rank| self.stale.search(rank).0);
        let below = (below < self.order.len()).then_some(below);
        let leaf = match (below, above) {
            (Some(below), Some(above)) if pos - above < below - pos => above,
            (Some(below), _) => below,
            (None, Some(above)) => above,
            (None, None) => return None,
        };
        let id = self.order[leaf];
        let at = self.slab[id as usize]
            .epochs
            .iter()
            .position(|e| *e != self.epoch)?;
        Some(Cur {
            leaf: id,
            at: small(at),
        })
    }

    fn new_leaf(&mut self, pos: u32) -> u32 {
        if let Some(id) = self.free.pop() {
            let leaf = &mut self.slab[id as usize];
            leaf.sum = 0;
            leaf.fresh = 0;
            leaf.fresh_epoch = 0;
            leaf.stale = false;
            leaf.pos = pos;
            id
        } else {
            small(self.slab.push(Leaf::new(pos)))
        }
    }

    /// Gives the leaves from `from` on their places, and rebuilds the sums.
    fn reorder(&mut self, from: usize) {
        for (pos, &id) in self.order.iter().enumerate().skip(from) {
            self.slab[id as usize].pos = small(pos);
        }
        let slab = &self.slab;
        self.sums
            .rebuild(self.order.iter().map(|&id| slab[id as usize].sum));
        self.counts
            .rebuild(self.order.iter().map(|&id| slab[id as usize].len() as u64));
        self.stale.rebuild(
            self.order
                .iter()
                .map(|&id| u64::from(slab[id as usize].stale)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    /// The same list kept the plain way, each item's id, key, height and whether it's fresh.
    type Plain = Vec<(ItemId, u64, u32, bool)>;

    fn check(index: &Index<u64>, plain: &Plain) {
        assert_eq!(index.len(), plain.len());
        let total: u64 = plain.iter().map(|(_, _, h, _)| u64::from(*h)).sum();
        assert_eq!(index.total(), total);
        assert_eq!(
            index.all_fresh(),
            plain.iter().all(|(_, _, _, fresh)| *fresh)
        );
        let mut before = 0u64;
        let mut cur = index.first();
        for (rank, (id, key, height, fresh)) in plain.iter().enumerate() {
            let at = cur.expect("an item for each of the plain list's");
            assert_eq!(index.key(at), key);
            assert_eq!(index.id(at), *id);
            assert_eq!(index.locate(*id), Some(at));
            assert_eq!(index.height(at), *height);
            assert_eq!(index.is_fresh(at), *fresh);
            assert_eq!(index.rank(at), rank);
            assert_eq!(index.offset_of(at), before);
            if *height > 0 {
                assert_eq!(index.find(before), Some((at, 0)));
                let last = u64::from(*height) - 1;
                assert_eq!(
                    index.find(before + last),
                    Some((at, u32::try_from(last).unwrap()))
                );
            }
            before += u64::from(*height);
            cur = index.next(at);
        }
        assert!(cur.is_none());
        // The nearest stale item is stale, and there is one unless all are fresh.
        if let Some(first) = index.first() {
            match index.next_stale(first) {
                Some(cur) => assert!(!index.is_fresh(cur)),
                None => assert!(index.all_fresh()),
            }
        }
        // Backwards too.
        let mut cur = index.last();
        for (_, key, _, _) in plain.iter().rev() {
            let at = cur.expect("an item for each of the plain list's");
            assert_eq!(index.key(at), key);
            cur = index.prev(at);
        }
        assert!(cur.is_none());
    }

    #[test]
    fn it_agrees_with_a_plain_list_through_random_edits() {
        for seed in 1..=40u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut index = Index::new();
            let mut plain: Plain = Vec::new();
            let mut gone = Vec::new();
            let mut next_key = 0u64;
            for step in 0..3000 {
                let roll = rng.below(100);
                if plain.is_empty() || roll < 35 {
                    let hint = u32::try_from(rng.below(9)).unwrap();
                    let id = index.push(next_key, hint);
                    plain.push((id, next_key, hint, false));
                    next_key += 1;
                } else if roll < 55 {
                    let at = usize::try_from(rng.below(plain.len() as u64)).unwrap();
                    let hint = u32::try_from(rng.below(9)).unwrap();
                    let cur = index.locate(plain[at].0).unwrap();
                    let id = index.insert(cur, next_key, hint);
                    plain.insert(at, (id, next_key, hint, false));
                    next_key += 1;
                } else if roll < 72 {
                    // Runs of removals empty whole leaves.
                    let at = usize::try_from(rng.below(plain.len() as u64)).unwrap();
                    let run = usize::try_from(rng.below(90))
                        .unwrap()
                        .min(plain.len() - at);
                    for _ in 0..run {
                        let cur = index.locate(plain[at].0).unwrap();
                        assert_eq!(index.remove(cur), plain[at].1);
                        gone.push(plain.remove(at).0);
                    }
                } else if roll < 85 {
                    let at = usize::try_from(rng.below(plain.len() as u64)).unwrap();
                    let height = u32::try_from(rng.below(50)).unwrap();
                    let cur = index.locate(plain[at].0).unwrap();
                    index.set(cur, height);
                    plain[at].2 = height;
                    plain[at].3 = true;
                } else if roll < 90 {
                    // A hint changes a height that isn't measured, and no other.
                    let at = usize::try_from(rng.below(plain.len() as u64)).unwrap();
                    let hint = u32::try_from(rng.below(30)).unwrap();
                    let cur = index.locate(plain[at].0).unwrap();
                    index.set_hint(cur, hint);
                    if !plain[at].3 {
                        plain[at].2 = hint;
                    }
                } else if roll < 97 {
                    let at = usize::try_from(rng.below(plain.len() as u64)).unwrap();
                    let cur = index.locate(plain[at].0).unwrap();
                    index.invalidate(cur);
                    plain[at].3 = false;
                } else {
                    index.bump_epoch();
                    for item in &mut plain {
                        item.3 = false;
                    }
                }
                if step % 97 == 0 {
                    check(&index, &plain);
                    // The ids of removed items find nothing, though their slots are reused.
                    for id in &gone {
                        assert_eq!(index.locate(*id), None);
                    }
                }
            }
            check(&index, &plain);
        }
    }

    #[test]
    fn the_nearest_stale_item_is_found_either_way() {
        let mut index = Index::new();
        let ids: Vec<ItemId> = (0..1000u64).map(|key| index.push(key, 1)).collect();
        let mut cur = index.first();
        while let Some(at) = cur {
            index.set(at, 2);
            cur = index.next(at);
        }
        assert!(index.all_fresh());
        let near = index.locate(ids[500]).unwrap();
        assert_eq!(index.next_stale(near), None);
        index.invalidate(index.locate(ids[3]).unwrap());
        index.invalidate(index.locate(ids[990]).unwrap());
        let found = index.next_stale(near).map(|cur| *index.key(cur));
        assert!(matches!(found, Some(3 | 990)));
        index.set(index.locate(ids[3]).unwrap(), 2);
        let found = index.next_stale(near).map(|cur| *index.key(cur));
        assert_eq!(found, Some(990));
        index.bump_epoch();
        let found = index.next_stale(near).map(|cur| *index.key(cur));
        assert!(
            found.is_some_and(|key| (448..512).contains(&key)),
            "{found:?}"
        );
    }

    #[test]
    fn a_slot_is_reused_under_a_new_generation() {
        let mut index = Index::new();
        let first = index.push("a", 1);
        let cur = index.locate(first).unwrap();
        assert_eq!(index.remove(cur), "a");
        let second = index.push("b", 1);
        assert_eq!(second.slot, first.slot);
        assert_ne!(second, first);
        assert_eq!(index.locate(first), None);
        assert_eq!(index.key(index.locate(second).unwrap()), &"b");
    }

    #[test]
    fn two_values_of_a_chunked_list_come_at_once() {
        let mut list: Chunked<u32, 4> = Chunked::new();
        for i in 0..10 {
            assert_eq!(list.push(i), i as usize);
        }
        for (a, b) in [(1, 2), (2, 1), (1, 9), (9, 1), (4, 7)] {
            let (x, y) = list.two_mut(a, b);
            assert_eq!(
                (*x, *y),
                (u32::try_from(a).unwrap(), u32::try_from(b).unwrap())
            );
        }
        assert_eq!(list.get(10), None);
    }

    #[test]
    fn leaves_split_across_chunks_keep_their_items() {
        let mut index = Index::new();
        let mut plain: Plain = Vec::new();
        for key in 0..(LEAF * LEAF_CHUNK * 2) as u64 {
            let id = index.push(key, 1);
            plain.push((id, key, 1, false));
        }
        // Inserts near the start split leaves in the first chunk into new leaves in the last.
        let first = plain.len() as u64;
        for (key, at) in (first..).zip((0..LEAF * 8).step_by(7)) {
            let cur = index.locate(plain[at].0).unwrap();
            let id = index.insert(cur, key, 1);
            plain.insert(at, (id, key, 1, false));
        }
        check(&index, &plain);
    }

    #[test]
    fn slots_past_one_chunk_find_their_items() {
        let mut index = Index::new();
        let ids: Vec<ItemId> = (0..3 * SLOT_CHUNK as u64)
            .map(|key| index.push(key, 1))
            .collect();
        for (key, id) in ids.iter().enumerate() {
            let cur = index.locate(*id).unwrap();
            assert_eq!(*index.key(cur), key as u64);
            assert_eq!(index.rank(cur), key);
        }
    }
}
