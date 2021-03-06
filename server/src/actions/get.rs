/*
 * Created on Fri Aug 14 2020
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

//! # `GET` queries
//! This module provides functions to work with `GET` queries

use crate::dbnet::connection::prelude::*;
use crate::resp::BytesWrapper;
use bytes::Bytes;

action!(
    /// Run a `GET` query
    fn get(handle: &crate::corestore::Corestore, con: &mut T, mut act: ActionIter) {
        err_if_len_is!(act, con, not 1);
        let res: Option<Bytes> = {
            let reader = kve!(con, handle);
            unsafe {
                // UNSAFE(@ohsayan): this is safe because we've already checked if the action
                // group contains one argument (excluding the action itself)
                match reader.get(act.next().unsafe_unwrap()) {
                    Ok(v) => v.map(|b| b.get_blob().clone()),
                    Err(_) => None,
                }
            }
        };
        if let Some(value) = res {
            // Good, we got the value, write it off to the stream
            con.write_response(BytesWrapper(value)).await?;
        } else {
            // Ah, couldn't find that key
            con.write_response(responses::groups::NIL).await?;
        }
        Ok(())
    }
);
