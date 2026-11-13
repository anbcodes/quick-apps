//! Children of one parent, kept by key, so a list can be updated in place
//! instead of rebuilt.

use super::*;

/// ```text
/// let mut rows = List::new(col);
/// let n = ui.text(&msg.text).detached();
/// rows.put(ui, msg.id.clone(), n); // appends, or replaces the old row
/// rows.remove(ui, &old_id);
/// ```
pub struct List<K> {
    pub parent: Node,
    items: Vec<(K, Node)>,
}

impl<K: PartialEq> List<K> {
    pub fn new(parent: Node) -> Self {
        List { parent, items: Vec::new() }
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn index(&self, k: &K) -> Option<usize> {
        self.items.iter().position(|(kk, _)| kk == k)
    }
    pub fn get(&self, k: &K) -> Option<Node> {
        self.items.iter().find(|(kk, _)| kk == k).map(|i| i.1)
    }
    /// The key whose node is `n`.
    pub fn key_of(&self, n: Node) -> Option<&K> {
        self.items.iter().find(|i| i.1 == n).map(|i| &i.0)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&K, Node)> {
        self.items.iter().map(|(k, n)| (k, *n))
    }
    /// Replaces the node for `k` if there is one, else appends it.
    pub fn put(&mut self, ui: &mut Ui, k: K, n: Node) {
        match self.index(&k) {
            Some(i) => {
                ui.replace(self.items[i].1, n);
                self.items[i].1 = n;
            }
            None => {
                ui.append(self.parent, n);
                self.items.push((k, n));
            }
        }
    }
    /// Inserts before position `at` (appends past the end). An existing
    /// node for `k` is destroyed first.
    pub fn insert(&mut self, ui: &mut Ui, at: usize, k: K, n: Node) {
        self.remove(ui, &k);
        match self.items.get(at) {
            Some(&(_, before)) => {
                ui.insert(self.parent, n, before);
                self.items.insert(at, (k, n));
            }
            None => {
                ui.append(self.parent, n);
                self.items.push((k, n));
            }
        }
    }
    /// Destroys the node for `k`.
    pub fn remove(&mut self, ui: &mut Ui, k: &K) -> bool {
        match self.index(k) {
            Some(i) => {
                ui.destroy(self.items.remove(i).1);
                true
            }
            None => false,
        }
    }
    /// Destroys every node.
    pub fn clear(&mut self, ui: &mut Ui) {
        for (_, n) in self.items.drain(..) {
            ui.destroy(n);
        }
    }
}
