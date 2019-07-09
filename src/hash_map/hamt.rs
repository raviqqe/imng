use super::bucket::{Bucket, BucketIterator};
use super::node::Node;
use std::borrow::Borrow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

const HASH_BITS: usize = 5;
const MAX_LEVEL: u8 = 64 / HASH_BITS as u8;
const NUM_ENTRIES: usize = 2 ^ HASH_BITS;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Entry<K, V> {
    Empty,
    KeyValue(K, V),
    HAMT(Arc<HAMT<K, V>>),
    Bucket(Arc<Bucket<K, V>>),
}

impl<K, V> Default for Entry<K, V> {
    fn default() -> Self {
        Entry::Empty
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HAMT<K, V> {
    // TODO: Use bitmap.
    level: u8,
    entries: [Entry<K, V>; NUM_ENTRIES],
}

impl<K: Clone + Hash + PartialEq, V: Clone> HAMT<K, V> {
    pub fn new(l: u8) -> Self {
        Self {
            level: l,
            entries: Default::default(),
        }
    }

    fn entry_index<Q: ?Sized + Hash + PartialEq>(&self, k: &Q) -> usize
    where
        K: Borrow<Q>,
    {
        ((hash(k) >> (self.level * 5)) & 0b11111) as usize
    }

    fn set_entry(&self, i: usize, e: Entry<K, V>) -> Self {
        let mut es = self.entries.clone();
        es[i] = e;

        Self {
            level: self.level,
            entries: es,
        }
    }

    #[cfg(test)]
    fn contain_bucket(&self) -> bool {
        self.entries.iter().any(|e| match e {
            Entry::Bucket(_) => true,
            _ => false,
        })
    }

    #[cfg(test)]
    fn is_normal(&self) -> bool {
        self.entries.iter().all(|e| match e {
            Entry::Bucket(b) => b.size() != 1,
            Entry::HAMT(h) => h.is_normal() && h.size() != 1,
            _ => true,
        })
    }
}

impl<K: Clone + Hash + PartialEq, V: Clone> Node<K, V> for HAMT<K, V> {
    fn insert(&self, k: K, v: V) -> (Self, bool) {
        let i = self.entry_index(&k);

        match &self.entries[i] {
            Entry::Empty => (self.set_entry(i, Entry::KeyValue(k, v)), true),
            Entry::KeyValue(kk, vv) => {
                if kk == &k {
                    (self.set_entry(i, Entry::KeyValue(k, v)), false)
                } else {
                    (
                        self.set_entry(
                            i,
                            if self.level < MAX_LEVEL {
                                Entry::HAMT(
                                    Self::new(self.level + 1)
                                        .insert(kk.clone(), vv.clone())
                                        .0
                                        .insert(k, v)
                                        .0
                                        .into(),
                                )
                            } else {
                                Entry::Bucket(
                                    Bucket::new(kk.clone(), vv.clone()).insert(k, v).0.into(),
                                )
                            },
                        ),
                        true,
                    )
                }
            }
            Entry::HAMT(h) => {
                let (h, new) = h.insert(k, v);
                (self.set_entry(i, Entry::HAMT(h.into())), new)
            }
            Entry::Bucket(b) => {
                let (b, new) = b.insert(k, v);
                (self.set_entry(i, Entry::Bucket(b.into())), new)
            }
        }
    }

    fn remove<Q: ?Sized + Hash + PartialEq>(&self, k: &Q) -> Option<Self>
    where
        K: Borrow<Q>,
    {
        let i = self.entry_index(k);

        Some(self.set_entry(
            i,
            match &self.entries[i] {
                Entry::Empty => return None,
                Entry::KeyValue(kk, _) => {
                    if kk.borrow() == k {
                        Entry::Empty
                    } else {
                        return None;
                    }
                }
                Entry::HAMT(h) => match h.remove(k) {
                    None => return None,
                    Some(h) => node_to_entry(&h, |h| Entry::HAMT(h.into())),
                },
                Entry::Bucket(b) => match b.remove(k) {
                    None => return None,
                    Some(b) => node_to_entry(&b, |b| Entry::Bucket(b.into())),
                },
            },
        ))
    }

    fn get<Q: ?Sized + Hash + PartialEq>(&self, k: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        match &self.entries[self.entry_index(k)] {
            Entry::Empty => None,
            Entry::KeyValue(kk, vv) => {
                if kk.borrow() == k {
                    Some(vv)
                } else {
                    None
                }
            }
            Entry::HAMT(h) => h.get(k),
            Entry::Bucket(b) => b.get(k),
        }
    }

    fn size(&self) -> usize {
        self.entries
            .iter()
            .map(|e| match e {
                Entry::Empty => 0,
                Entry::KeyValue(_, _) => 1,
                Entry::HAMT(h) => h.size(),
                Entry::Bucket(b) => b.size(),
            })
            .sum()
    }
}

fn node_to_entry<'a, K: 'a + Clone + Hash + PartialEq, V: 'a + Clone, N: Clone + Node<K, V>>(
    n: &'a N,
    f: fn(N) -> Entry<K, V>,
) -> Entry<K, V>
where
    &'a N: IntoIterator<Item = (&'a K, &'a V)>,
{
    if n.size() == 1 {
        n.into_iter()
            .next()
            .map(|(k, v)| Entry::KeyValue(k.clone(), v.clone()))
            .expect("non-empty node")
    } else {
        f(n.clone())
    }
}

fn hash<K: ?Sized + Hash>(k: &K) -> u64 {
    let mut h = DefaultHasher::new();
    k.hash(&mut h);
    h.finish()
}

#[derive(Clone, Debug)]
pub struct HAMTIterator<'a, K: 'a, V: 'a> {
    hamts: Vec<(&'a HAMT<K, V>, usize)>,
    bucket_iterator: Option<BucketIterator<'a, K, V>>,
}

impl<'a, K, V> IntoIterator for &'a HAMT<K, V> {
    type IntoIter = HAMTIterator<'a, K, V>;
    type Item = (&'a K, &'a V);

    fn into_iter(self) -> Self::IntoIter {
        HAMTIterator {
            hamts: vec![(self, 0)],
            bucket_iterator: None,
        }
    }
}

impl<'a, K, V> Iterator for HAMTIterator<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.bucket_iterator {
            Some(b) => b.next().or_else(|| {
                self.bucket_iterator = None;
                self.next()
            }),
            None => self.hamts.pop().and_then(|(h, i)| {
                if i == NUM_ENTRIES {
                    self.next()
                } else {
                    self.hamts.push((h, i + 1));

                    match &h.entries[i] {
                        Entry::Empty => self.next(),
                        Entry::HAMT(h) => {
                            self.hamts.push((h, 0));
                            self.next()
                        }
                        Entry::KeyValue(k, v) => Some((k, v)),
                        Entry::Bucket(b) => {
                            self.bucket_iterator = b.into_iter().into();
                            self.next()
                        }
                    }
                }
            }),
        }
    }
}

#[cfg(test)]
mod test {
    use super::super::node::Node;
    use super::{hash, HAMT, MAX_LEVEL};
    use rand::{random, seq::SliceRandom, thread_rng};
    use std::collections::{HashMap, HashSet};
    use test::Bencher;

    const NUM_ITERATIONS: usize = 1 << 12;

    #[test]
    fn new() {
        HAMT::new(0) as HAMT<usize, usize>;
    }

    #[test]
    fn insert() {
        let h = HAMT::new(0);

        assert_eq!(h.size(), 0);

        let (h, b) = h.insert(0, 0);

        assert!(b);
        assert_eq!(h.size(), 1);

        let (hh, b) = h.insert(0, 0);

        assert!(!b);
        assert_eq!(hh.size(), 1);

        let (h, b) = h.insert(1, 0);

        assert!(b);
        assert_eq!(h.size(), 2);
    }

    #[test]
    fn insert_many_in_order() {
        let mut h = HAMT::new(0);

        for i in 0..NUM_ITERATIONS {
            let (hh, b) = h.insert(i, i);
            h = hh;
            assert!(b);
            assert_eq!(h.size(), i + 1);
        }
    }

    #[test]
    fn insert_many_at_random() {
        let mut h: HAMT<usize, usize> = HAMT::new(0);

        for i in 0..NUM_ITERATIONS {
            let k = random();
            h = h.insert(k, k).0;
            assert_eq!(h.size(), i + 1);
        }
    }

    #[test]
    fn remove() {
        let h = HAMT::new(0);

        assert_eq!(h.insert(0, 0).0.remove(&0), Some(h.clone()));
        assert_eq!(h.insert(0, 0).0.remove(&1), None);
        assert_eq!(
            h.insert(0, 0).0.insert(1, 0).0.remove(&0),
            Some(h.insert(1, 0).0)
        );
        assert_eq!(
            h.insert(0, 0).0.insert(1, 0).0.remove(&1),
            Some(h.insert(0, 0).0)
        );
        assert_eq!(h.insert(0, 0).0.insert(1, 0).0.remove(&2), None);
    }

    #[test]
    fn insert_delete_many() {
        let mut h: HAMT<i16, i16> = HAMT::new(0);

        for _ in 0..NUM_ITERATIONS {
            let k = random();
            let s = h.size();
            let found = h.get(&k).is_some();

            if random() {
                h = h.insert(k, k).0;

                assert_eq!(h.size(), if found { s } else { s + 1 });
                assert_eq!(h.get(&k), Some(&k));
            } else {
                h = h.remove(&k).unwrap_or(h);

                assert_eq!(h.size(), if found { s - 1 } else { s });
                assert_eq!(h.get(&k), None);
            }

            assert!(h.is_normal());
        }
    }

    #[test]
    fn get() {
        let h = HAMT::new(0);

        assert_eq!(h.insert(0, 0).0.get(&0), Some(&0));
        assert_eq!(h.insert(0, 0).0.get(&1), None);
        assert_eq!(h.insert(1, 0).0.get(&0), None);
        assert_eq!(h.insert(1, 0).0.get(&1), Some(&0));
        assert_eq!(h.insert(0, 0).0.insert(1, 0).0.get(&0), Some(&0));
        assert_eq!(h.insert(0, 0).0.insert(1, 0).0.get(&1), Some(&0));
        assert_eq!(h.insert(0, 0).0.insert(1, 0).0.get(&2), None);
    }

    #[test]
    fn equality() {
        for _ in 0..8 {
            let mut hs: [HAMT<i16, i16>; 2] = [HAMT::new(0), HAMT::new(0)];
            let mut is: Vec<i16> = (0..NUM_ITERATIONS).map(|_| random()).collect();
            let mut ds: Vec<i16> = (0..NUM_ITERATIONS).map(|_| random()).collect();

            for h in hs.iter_mut() {
                is.shuffle(&mut thread_rng());
                ds.shuffle(&mut thread_rng());

                for i in &is {
                    *h = h.insert(*i, *i).0;
                }

                for d in &ds {
                    *h = h.remove(&d).unwrap_or(h.clone());
                }
            }

            assert_eq!(hs[0], hs[1]);
        }
    }

    #[test]
    fn collision() {
        let mut h = HAMT::new(MAX_LEVEL);
        let mut s = HashSet::new();

        for k in 0.. {
            assert!(!h.contain_bucket());

            h = h.insert(k, k).0;

            let i = hash(&k) >> 60;

            if s.contains(&i) {
                break;
            }

            s.insert(i);
        }

        assert!(h.contain_bucket());
    }

    #[test]
    fn iterator() {
        let mut ss: Vec<usize> = (0..42).collect();

        for _ in 0..100 {
            ss.push(random::<usize>() % 1024);
        }

        for l in vec![0, MAX_LEVEL] {
            for s in &ss {
                let mut h: HAMT<i16, i16> = HAMT::new(l);
                let mut m: HashMap<i16, i16> = HashMap::new();

                for _ in 0..*s {
                    let k = random();
                    let v = random();

                    let (hh, _) = h.insert(k, v);
                    h = hh;

                    m.insert(k, v);
                }

                let mut s = 0;

                for (k, v) in h.into_iter() {
                    s += 1;

                    assert_eq!(m[k], *v);
                }

                assert_eq!(s, h.size());
            }
        }
    }

    #[test]
    fn iterate_with_buckets() {
        let ks = (0..1000).collect::<Vec<_>>();

        let mut h = HAMT::new(MAX_LEVEL);

        for k in &ks {
            h = h.insert(k, k).0;
        }

        assert_eq!(ks.len(), h.into_iter().collect::<Vec<_>>().len())
    }

    fn keys() -> Vec<i16> {
        (0..1000).collect()
    }

    #[bench]
    fn bench_insert_1000(b: &mut Bencher) {
        let ks = keys();

        b.iter(|| {
            let mut h = HAMT::new(0);

            for k in &ks {
                h = h.insert(k, k).0;
            }
        });
    }

    #[bench]
    fn bench_get_1000(b: &mut Bencher) {
        let ks = keys();
        let mut h = HAMT::new(0);

        for k in &ks {
            h = h.insert(k, k).0;
        }

        b.iter(|| {
            for k in &ks {
                h.get(&k);
            }
        });
    }

    #[bench]
    fn bench_hash_map_insert_1000(b: &mut Bencher) {
        let ks = keys();

        b.iter(|| {
            let mut h = HashMap::new();

            for k in &ks {
                h.insert(k, k);
            }
        });
    }

    #[bench]
    fn bench_hash_map_get_1000(b: &mut Bencher) {
        let ks = keys();
        let mut h = HashMap::new();

        for k in &ks {
            h.insert(k, k);
        }

        b.iter(|| {
            for k in &ks {
                h.get(&k);
            }
        });
    }
}
