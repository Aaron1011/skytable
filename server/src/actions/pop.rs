/*
 * Created on Mon Jun 14 2021
 *
 * This file is a part of Skytable
 * Skytable (formerly known as TerrabaseDB or Skybase) is a free and open-source
 * NoSQL database written by Sayan Nandan ("the Author") with the
 * vision to provide flexibility in data modelling without compromising
 * on performance, queryability or scalability.
 *
 * Copyright (c) 2021, Sayan Nandan <ohsayan@outlook.com>
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

use crate::corestore;
use crate::dbnet::connection::prelude::*;
use crate::protocol::responses;
use crate::queryengine::ActionIter;
use crate::resp::BytesWrapper;

action!(
    /// Run a POP action
    fn pop(handle: &corestore::Corestore, con: &mut T, act: ActionIter) {
        err_if_len_is!(act, con, eq 0);
        if registry::state_okay() {
            con.write_array_length(act.len()).await?;
            for key in act {
                if !registry::state_okay() {
                    // we keep this check just in case the server fails in-between running a
                    // pop operation
                    con.write_response(responses::groups::SERVER_ERR).await?;
                } else {
                    match kve!(con, handle).pop(key) {
                        Ok(Some((_key, val))) => {
                            con.write_response(BytesWrapper(val.into_inner())).await?
                        }
                        Ok(None) => con.write_response(responses::groups::NIL).await?,
                        Err(_) => {
                            con.write_response(responses::groups::ENCODING_ERROR)
                                .await?
                        }
                    }
                }
            }
        } else {
            // don't begin the operation at all if the database is poisoned
            return con.write_response(responses::groups::SERVER_ERR).await;
        }
        Ok(())
    }
);
