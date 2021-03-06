/*
 * Created on Thu Oct 01 2020
 *
 * This file is a part of Skytable
 * Skytable (formerly known as TerrabaseDB or Skybase) is a free and open-source
 * NoSQL database written by Sayan Nandan ("the Author") with the
 * vision to provide flexibility in data modelling without compromising
 * on performance, queryability or scalability.
 *
 * Copyright (c) 2020, Sayan Nandan <ohsayan@outlook.com>
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 *
*/

//! Tools for creating snapshots

use crate::corestore::lazy::Lazy;
use crate::corestore::Corestore;
use crate::storage;
use crate::storage::interface::DIR_SNAPROOT;
use chrono::prelude::*;
use regex::Regex;
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind};

/// Matches any string which is in the following format:
/// ```text
/// YYYYMMDD-HHMMSS
/// ```
pub static SNAP_MATCH: Lazy<Regex, fn() -> Regex> = Lazy::new(|| {
    Regex::new("^\\d{4}(0[1-9]|1[012])(0[1-9]|[12][0-9]|3[01])(-)(?:(?:([01]?\\d|2[0-3]))?([0-5]?\\d))?([0-5]?\\d)$").unwrap()
});

/// The default snapshot count is 12, assuming that the user would take a snapshot
/// every 2 hours (or 7200 seconds)
const DEF_SNAPSHOT_COUNT: usize = 12;

/// # Snapshot Engine
///
/// This object provides methods to create and delete snapshots. There should be a
/// `snapshot_scheduler` which should hold an instance of this object, on startup.
/// Whenever the duration expires, the caller should call `mksnap()`
pub struct SnapshotEngine<'a> {
    /// File names of the snapshots (relative paths)
    snaps: queue::Queue,
    /// An atomic reference to the coretable
    dbref: &'a Corestore,
}

#[derive(Debug)]
pub enum SnapengineError {
    EngineError(&'static str),
    IoError(io::Error),
}

impl fmt::Display for SnapengineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), fmt::Error> {
        match self {
            Self::EngineError(estr) => {
                formatter.write_str("Snapshot engine error")?;
                formatter.write_str(estr)?;
            }
            Self::IoError(e) => {
                formatter.write_str("Snapshot engine IOError:")?;
                formatter.write_str(&e.to_string())?;
            }
        }
        Ok(())
    }
}

impl<'a> SnapshotEngine<'a> {
    /// Create a new `Snapshot` instance
    ///
    /// This also attempts to check if the snapshots directory exists;
    /// If the directory doesn't exist, then it is created
    pub fn new<'b: 'a>(maxtop: usize, dbref: &'b Corestore) -> Result<Self, SnapengineError> {
        let mut snaps = Vec::with_capacity(maxtop);
        let q_cfg_tuple = if maxtop == 0 {
            (DEF_SNAPSHOT_COUNT, true)
        } else {
            (maxtop, false)
        };
        match fs::create_dir(DIR_SNAPROOT) {
            Ok(_) => (),
            Err(e) => match e.kind() {
                ErrorKind::AlreadyExists => {
                    // Now it's our turn to look for the existing snapshots
                    let dir = fs::read_dir(DIR_SNAPROOT).map_err(SnapengineError::IoError)?;
                    for entry in dir {
                        let entry = entry.map_err(SnapengineError::IoError)?;
                        let path = entry.path();
                        // We'll skip the directory that contains remotely created snapshots
                        if path.is_file() {
                            // If the entry is not a directory then some other
                            // file(s) is present in the directory
                            println!("Erroring at: {:?}", path);
                            return Err(SnapengineError::EngineError(
                                "The snapshot directory contains unrecognized files/directories",
                            ));
                        }
                        if !path.is_dir() {
                            let fname = entry.file_name();
                            let file_name = if let Some(good_file_name) = fname.to_str() {
                                good_file_name
                            } else {
                                // The filename contains invalid characters
                                return Err(SnapengineError::EngineError(
                                "The snapshot file names have invalid characters. This should not happen! Please report an error")
                            );
                            };
                            if SNAP_MATCH.is_match(file_name) {
                                // Good, the file name matched the format we were expecting
                                // This is a valid snapshot, add it to our `Vec` of snaps
                                snaps.push(file_name.to_owned());
                            } else {
                                // The filename contains invalid characters
                                return Err(SnapengineError::EngineError(
                                "The snapshot file names have invalid characters. This should not happen! Please report an error"
                            ));
                            }
                        }
                    }
                    if snaps.is_empty() {
                        return Ok(SnapshotEngine {
                            snaps: queue::Queue::new(q_cfg_tuple),
                            dbref,
                        });
                    } else {
                        return Ok(SnapshotEngine {
                            snaps: queue::Queue::init_pre(q_cfg_tuple, snaps),
                            dbref,
                        });
                    }
                }
                _ => return Err(SnapengineError::IoError(e)),
            },
        }
        Ok(SnapshotEngine {
            snaps: queue::Queue::new(q_cfg_tuple),
            dbref,
        })
    }
    /// Generate the snapshot name
    fn get_snapname(&self) -> String {
        Utc::now().format("%Y%m%d-%H%M%S").to_string()
    }
    pub fn _mksnap_nonblocking_section(&mut self) -> (String, Option<String>) {
        let snapname = self.get_snapname();
        let old_snap_if_any = self.snaps.add(snapname.clone());
        (snapname, old_snap_if_any)
    }

    /// Blocking section of the snapshotting process
    ///
    /// This is the blocking section of the snapshot process that requires slow disk I/O. This has been logically
    /// separated for the `Self::mksnap()` async task that will spawn this blocking section on the runtime's
    /// dedicated thread for performing blocking operations
    pub(in crate::diskstore::snapshot) fn mksnap_blocking_section(
        snapname: String,
        handle: Corestore,
        oldsnap: Option<String>,
    ) -> bool {
        // This is a potentially blocking section
        // So we acquired a lock
        let lck = handle.lock_snap(); // Lock the snapshot service
                                      // Another blocking section that does the actual I/O
        if let Err(e) = storage::flush::snap_flush_full(&snapname, handle.get_store()) {
            log::error!("Snapshotting failed with error: '{}'", e);
            drop(lck);
            return false;
        } else {
            log::info!("Successfully created snapshot");
        }
        if let Some(old_snapshot) = oldsnap {
            if let Err(e) = fs::remove_dir_all(crate::concat_str!(DIR_SNAPROOT, "/", &old_snapshot))
            {
                log::error!(
                    "Failed to delete snapshot '{}' with error '{}'",
                    old_snapshot,
                    e
                );
                drop(lck);
                return false;
            } else {
                log::info!("Successfully removed old snapshot");
            }
        }
        drop(lck);
        true
    }
    /// Create a snapshot
    ///
    /// This returns `true` if everything went well, otherwise it returns
    /// `false`.
    ///
    /// ## Nature
    ///
    /// This function is **blocking in nature** since it waits for the snapshotting service
    /// to be free. It's best to check if the snapshotting service is busy by using the function `corestore.snapcfg.is_busy()`
    ///
    ///
    /// ## Panics
    /// If snapshotting is disabled in `Corestore` then this will panic badly! It
    /// may not even panic: but terminate abruptly with `SIGILL`. This service will also panic in the case
    /// of a runtime error.
    pub async fn mksnap(&mut self) -> bool {
        let (create_this, remove_this) = self._mksnap_nonblocking_section();
        let owned_handle = self.dbref.clone();
        tokio::task::spawn_blocking(move || {
            SnapshotEngine::mksnap_blocking_section(create_this, owned_handle, remove_this)
        })
        .await
        .expect("MKSNAP INTERNAL SERVICE PANIC")
    }
}

mod queue {
    //! An extremely simple queue implementation which adds more items to the queue
    //! freely and once the threshold limit is reached, it pops off the oldest element and returns it
    //!
    //! This implementation is specifically built for use with the snapshotting utility
    #[derive(Debug, PartialEq)]
    pub struct Queue {
        queue: Vec<String>,
        maxlen: usize,
        dontpop: bool,
    }
    impl Queue {
        pub fn new((maxlen, dontpop): (usize, bool)) -> Self {
            Queue {
                queue: Vec::with_capacity(maxlen),
                maxlen,
                dontpop,
            }
        }
        pub const fn init_pre((maxlen, dontpop): (usize, bool), queue: Vec<String>) -> Self {
            Queue {
                queue,
                maxlen,
                dontpop,
            }
        }
        /// This returns a `String` only if the queue is full. Otherwise, a `None` is returned most of the time
        pub fn add(&mut self, item: String) -> Option<String> {
            if self.dontpop {
                // We don't need to pop anything since the user
                // wants to keep all the items in the queue
                self.queue.push(item);
                None
            } else {
                // The user wants to keep a maximum of `maxtop` items
                // so we will check if the current queue is full
                // if it is full, then the `maxtop` limit has been reached
                // so we will remove the oldest item and then push the
                // new item onto the queue
                let x = if self.is_overflow() { self.pop() } else { None };
                self.queue.push(item);
                x
            }
        }
        /// Check if we have reached the maximum queue size limit
        fn is_overflow(&self) -> bool {
            self.queue.len() == self.maxlen
        }
        /// Remove the last item inserted
        fn pop(&mut self) -> Option<String> {
            if self.queue.is_empty() {
                None
            } else {
                Some(self.queue.remove(0))
            }
        }
    }

    #[test]
    fn test_queue() {
        let mut q = Queue::new((4, false));
        assert!(q.add(String::from("snap1")).is_none());
        assert!(q.add(String::from("snap2")).is_none());
        assert!(q.add(String::from("snap3")).is_none());
        assert!(q.add(String::from("snap4")).is_none());
        assert_eq!(q.add(String::from("snap5")), Some(String::from("snap1")));
        assert_eq!(q.add(String::from("snap6")), Some(String::from("snap2")));
    }

    #[test]
    fn test_queue_dontpop() {
        // This means that items can only be added or all of them can be deleted
        let mut q = Queue::new((4, true));
        assert!(q.add(String::from("snap1")).is_none());
        assert!(q.add(String::from("snap2")).is_none());
        assert!(q.add(String::from("snap3")).is_none());
        assert!(q.add(String::from("snap4")).is_none());
        assert!(q.add(String::from("snap5")).is_none());
        assert!(q.add(String::from("snap6")).is_none());
    }
}
