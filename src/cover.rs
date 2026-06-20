use crate::types::*;
use std::collections::{HashMap, HashSet};

pub struct GreedyCover {
    /// All objects: object_id → (attribute → value)
    pub objects: HashMap<ObjectId, HashMap<String, String>>,
    /// The set of all property atoms (e.g. "Role=Admin")
    pub atoms: Vec<PropertyAtom>,
    /// Binary relation: set of (object_id, property_atom) pairs
    pub relation: HashSet<(ObjectId, PropertyAtom)>,
    /// Pairs not yet covered by any factor
    pub uncovered: HashSet<(ObjectId, PropertyAtom)>,
    /// Inverted index: atom → set of object IDs with that atom still uncovered.
    /// Maintained in sync with `uncovered` for O(1) extent lookup per atom.
    atom_to_objs: HashMap<PropertyAtom, HashSet<ObjectId>>,
}

impl GreedyCover {
    /// Initialise from a list of objects (object_id, properties)
    pub fn new(objects: Vec<(ObjectId, HashMap<String, String>)>) -> Self {
        let mut all_objects = HashMap::new();
        let mut atoms_set = HashSet::new();
        let mut relation = HashSet::new();
        let mut atom_to_objs: HashMap<PropertyAtom, HashSet<ObjectId>> = HashMap::new();

        for (oid, props) in objects {
            for (attr, val) in &props {
                let atom = format!("{}={}", attr, val);
                atoms_set.insert(atom.clone());
                relation.insert((oid, atom.clone()));
                atom_to_objs.entry(atom).or_default().insert(oid);
            }
            all_objects.insert(oid, props);
        }

        let atoms: Vec<_> = atoms_set.into_iter().collect();
        let uncovered = relation.clone();

        GreedyCover {
            objects: all_objects,
            atoms,
            relation,
            uncovered,
            atom_to_objs,
        }
    }

    /// Find the largest rectangle that covers uncovered pairs.
    /// Returns (extent, intent) or None if nothing left.
    ///
    /// Uses the inverted index to avoid the O(|uncovered|) linear scan per atom.
    /// Cost: O(|atoms| + |atoms| × |best_extent|) per call instead of O(|atoms| × |uncovered|).
    fn largest_rectangle(&self) -> Option<(HashSet<ObjectId>, Vec<PropertyAtom>)> {
        if self.uncovered.is_empty() {
            return None;
        }

        // Find the atom with the most uncovered objects — O(|atoms|) via inverted index.
        let (base_atom, base_set) = self.atom_to_objs.iter()
            .filter(|(_, s)| !s.is_empty())
            .max_by_key(|(_, s)| s.len())?;

        let extent: HashSet<ObjectId> = base_set.clone();
        let mut intent = vec![base_atom.clone()];

        // Try to extend intent: add any atom shared by ALL objects in extent.
        // O(|atoms| × |extent|) total, short-circuits early.
        for atom in &self.atoms {
            if atom == base_atom {
                continue;
            }
            if let Some(objs) = self.atom_to_objs.get(atom) {
                if extent.iter().all(|oid| objs.contains(oid)) {
                    intent.push(atom.clone());
                }
            }
        }

        Some((extent, intent))
    }

    /// Build factors greedily until everything is covered.
    pub fn build_factors(&mut self) -> Vec<Factor> {
        let mut factors = Vec::new();
        let mut next_id: u64 = 1;

        while let Some((extent, intent)) = self.largest_rectangle() {
            // Remove covered pairs from uncovered and from the inverted index.
            for oid in &extent {
                for atom in &intent {
                    if self.uncovered.remove(&(*oid, atom.clone())) {
                        if let Some(objs) = self.atom_to_objs.get_mut(atom) {
                            objs.remove(oid);
                        }
                    }
                }
            }

            factors.push(Factor {
                id: next_id,
                extent,
                intent,
                is_structural: true,
                access_count: 0,
                created_at: String::new(),
                last_accessed: String::new(),
            });
            next_id += 1;
        }

        // Any leftover pairs become singleton factors.
        while !self.uncovered.is_empty() {
            let pair = self.uncovered.iter().next().cloned().unwrap();
            self.uncovered.remove(&pair);
            if let Some(objs) = self.atom_to_objs.get_mut(&pair.1) {
                objs.remove(&pair.0);
            }

            factors.push(Factor {
                id: next_id,
                extent: std::iter::once(pair.0).collect(),
                intent: vec![pair.1],
                is_structural: true,
                access_count: 0,
                created_at: String::new(),
                last_accessed: String::new(),
            });
            next_id += 1;
        }

        factors
    }
}
