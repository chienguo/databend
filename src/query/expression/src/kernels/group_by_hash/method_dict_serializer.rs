// Copyright 2021 Datafuse Labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::ptr::NonNull;

use databend_common_exception::Result;
use databend_common_hashtable::DictionaryKeys;
use databend_common_hashtable::FastHash;
use either::Either;

use super::utils::serialize_group_columns;
use crate::Column;
use crate::HashMethod;
use crate::InputColumns;
use crate::KeyAccessor;
use crate::KeysState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashMethodDictionarySerializer {
    pub dict_keys: usize,
}

impl HashMethod for HashMethodDictionarySerializer {
    type HashKey = DictionaryKeys;
    type HashKeyIter<'a> = std::slice::Iter<'a, DictionaryKeys>;

    fn name(&self) -> String {
        "DictionarySerializer".to_string()
    }

    fn build_keys_state(&self, group_columns: InputColumns, num_rows: usize) -> Result<KeysState> {
        // fixed type serialize one column to dictionary
        let mut dictionary_columns = Vec::with_capacity(group_columns.len());
        let mut serialize_columns = Vec::new();
        for group_column in group_columns.iter() {
            match group_column {
                Column::Binary(v) | Column::Variant(v) | Column::Bitmap(v) => {
                    debug_assert_eq!(v.len(), num_rows);
                    dictionary_columns.push(Either::Right(v.clone()));
                }
                Column::String(v) => {
                    debug_assert_eq!(v.len(), num_rows);
                    dictionary_columns.push(Either::Left(v.clone()));
                }
                _ => serialize_columns.push(group_column.clone()),
            }
        }

        if !serialize_columns.is_empty() {
            // The serialize_size is equal to the number of bytes required by serialization.
            let mut serialize_size = 0;
            for column in serialize_columns.iter() {
                serialize_size += column.serialize_size();
            }
            let state =
                serialize_group_columns((&serialize_columns).into(), num_rows, serialize_size);
            dictionary_columns.push(Either::Right(state));
        }

        let mut keys = Vec::with_capacity(num_rows * dictionary_columns.len());
        let mut points = Vec::with_capacity(num_rows * dictionary_columns.len());

        for row in 0..num_rows {
            let start = points.len();

            for dictionary_column in &dictionary_columns {
                let data = match dictionary_column {
                    Either::Left(l) => unsafe { l.index_unchecked(row).as_bytes() },
                    Either::Right(r) => unsafe { r.index_unchecked(row) },
                };
                points.push(NonNull::from(data));
            }

            keys.push(DictionaryKeys::create(&points[start..]))
        }

        Ok(KeysState::Dictionary {
            dictionaries: keys,
            keys_point: points,
            columns: dictionary_columns,
        })
    }

    fn build_keys_iter<'a>(&self, keys_state: &'a KeysState) -> Result<Self::HashKeyIter<'a>> {
        match keys_state {
            KeysState::Dictionary { dictionaries, .. } => Ok(dictionaries.iter()),
            _ => unreachable!(),
        }
    }

    fn build_keys_accessor(
        &self,
        keys_state: KeysState,
    ) -> Result<Box<dyn KeyAccessor<Key = Self::HashKey>>> {
        match keys_state {
            KeysState::Dictionary { dictionaries, .. } => {
                Ok(Box::new(DicKeyAccessor::new(dictionaries)))
            }
            _ => unreachable!(),
        }
    }

    fn build_keys_hashes(&self, keys_state: &KeysState, hashes: &mut Vec<u64>) {
        match keys_state {
            KeysState::Dictionary { dictionaries, .. } => {
                hashes.extend(dictionaries.iter().map(|key| key.fast_hash()));
            }
            _ => unreachable!(),
        }
    }
}

pub struct DicKeyAccessor {
    data: Vec<DictionaryKeys>,
}

impl DicKeyAccessor {
    pub fn new(data: Vec<DictionaryKeys>) -> Self {
        Self { data }
    }
}

impl KeyAccessor for DicKeyAccessor {
    type Key = DictionaryKeys;

    /// # Safety
    /// Calling this method with an out-of-bounds index is *[undefined behavior]*.
    unsafe fn key_unchecked(&self, index: usize) -> &Self::Key {
        self.data.get_unchecked(index)
    }
}
