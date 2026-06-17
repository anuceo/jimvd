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
}

impl GreedyCover {
    /// Initialise from a list of objects (object_id, properties)
    pub fn new(objects: Vec<(ObjectId, HashMap<String, String>)>) -> Self {
        let mut all_objects = HashMap::new();
        let mut atoms_set = HashSet::new();
        let mut relation = HashSet::new();

        for (oid, props) in objects {
            for (attr, val) in &props {
                let atom = format!("{}={}", attr, val);
                atoms_set.insert(atom.clone());
                relation.insert((oid, atom));
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
        }
    }

    /// Find the largest rectangle that covers uncovered pairs.
    /// Returns (extent, intent) or None if nothing left.
    fn largest_rectangle(&self) -> Option<(Vec<ObjectId>, Vec<PropertyAtom>)> {
        if self.uncovered.is_empty() {
            return None;
        }

        // Greedy: for each atom, compute its uncovered extent.
        // The rectangle is (extent of most frequent atom, that atom alone).
        // Then we'll try to enlarge the intent by adding other atoms that are common to the same extent.
        let mut best_atom: Option<&PropertyAtom> = None;
        let mut best_extent = Vec::new();
        let mut best_size = 0;

        for atom in &self.atoms {
            let extent: Vec<ObjectId> = self.uncovered
                .iter()
                .filter(|(_, a)| a == atom)
                .map(|(oid, _)| *oid)
                .collect();
            if extent.len() > best_size {
                best_size = extent.len();
                best_atom = Some(atom);
                best_extent = extent;
            }
        }

        let base_atom = best_atom?;
        let extent: HashSet<ObjectId> = best_extent.into_iter().collect();
        let mut intent = vec![base_atom.clone()];

        // Try to add other atoms that are shared by all objects in the current extent
        for atom in &self.atoms {
            if atom == base_atom {
                continue;
            }
            // Check if every object in extent has this atom (according to uncovered)
            if extent.iter().all(|oid| self.uncovered.contains(&(*oid, atom.clone()))) {
                intent.push(atom.clone());
            }
        }

        // Remove the newly covered pairs from uncovered (we'll do that in build_factors)
        Some((extent.into_iter().collect(), intent))
    }

    /// Build factors greedily until everything is covered.
    pub fn build_factors(&mut self) -> Vec<Factor> {
        let mut factors = Vec::new();
        let mut next_id: u64 = 1;

        while let Some((extent, intent)) = self.largest_rectangle() {
            // Remove covered pairs from uncovered
            for oid in &extent {
                for atom in &intent {
                    self.uncovered.remove(&(*oid, atom.clone()));
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

        // Any leftover pairs? Each becomes a singleton factor (one object, one property).
        while !self.uncovered.is_empty() {
            let pair = self.uncovered.iter().next().cloned().unwrap();
            self.uncovered.remove(&pair);

            factors.push(Factor {
                id: next_id,
                extent: vec![pair.0],
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