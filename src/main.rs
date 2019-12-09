// Copyright 2019 Parity Technologies (UK) Ltd.
// This file is part of Parity-Bridge.

// Parity-Bridge is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity-Bridge is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity-Bridge.  If not, see <http://www.gnu.org/licenses/>.

#![recursion_limit="1024"]

mod ethereum_client;
mod ethereum_headers;
mod ethereum_sync;
mod ethereum_types;
mod substrate_client;
mod substrate_types;

use crate::ethereum_types::HeaderStatus as EthereumHeaderStatus;

// TODO: when SharedClient will be available, switch to Substrate headers subscription
// (because we do not need old Substrate headers)

/// Interval (in ms) at which we check new Ethereum headers when we are synced/almost synced.
const ETHEREUM_TICK_INTERVAL_MS: u64 = 10_000;
/// Interval (in ms) at which we check new Substrate blocks.
const SUBSTRATE_TICK_INTERVAL_MS: u64 = 5_000;
/// When we submit Ethereum headers to Substrate runtime, but see no updates of best
/// Ethereum block known to Substrate runtime during STALL_SYNC_TIMEOUT_MS milliseconds,
/// we consider that our headers are rejected because there has been reorg in Substrate.
/// This reorg could invalidate oud knowledge about sync process (i.e. we have asked if
/// HeaderA is known to Substrate, but then reorg happened and the answer is different
/// now) => we need to reset sync.
/// The other option is to receive **EVERY** best Substrate header and check if it is
/// direct child of previous best header. But: (1) subscription doesn't guarantee that
// the subscriber will receive every best header (2) reorg won't always lead to sync
/// stall and restart is a heavy operation (we forget all in-memory headers).
const STALL_SYNC_TIMEOUT_MS: u64 = 30_000;

async fn delay(timeout_ms: u64) {
	async_std::task::sleep(std::time::Duration::from_millis(timeout_ms)).await
}

fn interval(timeout_ms: u64) -> impl futures::Stream<Item = ()> {
	futures::stream::unfold((), move |_| async move { delay(timeout_ms).await; Some(((), ())) })
}

fn print_progress(
	progress_context: (std::time::Instant, Option<u64>, Option<u64>),
	eth_sync: &crate::ethereum_sync::HeadersSync,
) -> (std::time::Instant, Option<u64>, Option<u64>) {
	let (prev_time, prev_best_header, prev_target_header) = progress_context;
	let now_time = std::time::Instant::now();
	let (now_best_header, now_target_header) = eth_sync.status();

	let need_update = now_time - prev_time > std::time::Duration::from_secs(10)
		|| match (prev_best_header, now_best_header) {
			(Some(prev_best_header), Some(now_best_header)) => now_best_header.0.saturating_sub(prev_best_header) > 10,
			_ => false,
		};
	if !need_update {
		return (prev_time, prev_best_header, prev_target_header);
	}

	log::info!(
		target: "bridge",
		"Synced {:?} of {:?} headers",
		now_best_header.map(|id| id.0),
		now_target_header,
	);
	(now_time, now_best_header.clone().map(|id| id.0), *now_target_header)
}

fn main() {
	use futures::{future::FutureExt, stream::StreamExt};

	env_logger::init();

	let mut local_pool = futures::executor::LocalPool::new();
	let mut progress_context = (std::time::Instant::now(), None, None);

	local_pool.run_until(async move {
		let mut eth_sync = crate::ethereum_sync::HeadersSync::default();
		let mut stall_countdown = None;

		let mut eth_maybe_client = None;
		let mut eth_best_block_number_required = false;
		let eth_best_block_number_future = ethereum_client::best_block_number(ethereum_client::client()).fuse();
		let eth_new_header_future = futures::future::Fuse::terminated();
		let eth_orphan_header_future = futures::future::Fuse::terminated();
		let eth_receipts_future = futures::future::Fuse::terminated();
		let eth_tick_stream = interval(ETHEREUM_TICK_INTERVAL_MS).fuse();

		let mut sub_maybe_client = None;
		let mut sub_best_block_required = false;
		let sub_best_block_future = substrate_client::best_ethereum_block(substrate_client::client()).fuse();
		let sub_receipts_check_future = futures::future::Fuse::terminated();
		let sub_existence_status_future = futures::future::Fuse::terminated();
		let sub_submit_header_future = futures::future::Fuse::terminated();
		let sub_tick_stream = interval(SUBSTRATE_TICK_INTERVAL_MS).fuse();

		futures::pin_mut!(
			eth_best_block_number_future,
			eth_new_header_future,
			eth_orphan_header_future,
			eth_receipts_future,
			eth_tick_stream,
			sub_best_block_future,
			sub_receipts_check_future,
			sub_existence_status_future,
			sub_submit_header_future,
			sub_tick_stream
		);

		loop {
			futures::select! {
				(eth_client, eth_best_block_number) = eth_best_block_number_future => {
					eth_maybe_client = Some(eth_client);
					eth_best_block_number_required = false;

					match eth_best_block_number {
						Ok(eth_best_block_number) => eth_sync
							.ethereum_best_header_number_response(eth_best_block_number),
						Err(error) => log::error!(
							target: "bridge",
							"Error retrieving best header number from Ethereum number: {:?}",
							error,
						),
					}
				},
				(eth_client, eth_new_header) = eth_new_header_future => {
					eth_maybe_client = Some(eth_client);

					match eth_new_header {
						Ok(eth_new_header) => eth_sync.headers_mut().header_response(eth_new_header),
						Err(error) => log::error!(
							target: "bridge",
							"Error retrieving header from Ethereum node: {:?}",
							error,
						),
					}
				},
				(eth_client, eth_orphan_header) = eth_orphan_header_future => {
					eth_maybe_client = Some(eth_client);

					match eth_orphan_header {
						Ok(eth_orphan_header) => eth_sync.headers_mut().header_response(eth_orphan_header),
						Err(error) => log::error!(
							target: "bridge",
							"Error retrieving header from Ethereum node: {:?}",
							error,
						),
					}
				},
				(eth_client, eth_receipts) = eth_receipts_future => {
					eth_maybe_client = Some(eth_client);

					match eth_receipts {
						Ok((header, receipts)) => eth_sync.headers_mut().receipts_response(&header, receipts),
						Err(error) => log::error!(
							target: "bridge",
							"Error retrieving transactions receipts from Ethereum node: {:?}",
							error,
						),
					}
				},
				_ = eth_tick_stream.next() => {
					if eth_sync.is_almost_synced() {
						eth_best_block_number_required = true;
					}
				},
				(sub_client, sub_best_block) = sub_best_block_future => {
					sub_maybe_client = Some(sub_client);
					sub_best_block_required = false;

					match sub_best_block {
						Ok(sub_best_block) => {
							let head_updated = eth_sync.substrate_best_header_response(sub_best_block);
							match head_updated {
								// IF head is updated AND there are still our transactions:
								// => restart stall countdown timer
								true if eth_sync.headers().headers_in_status(EthereumHeaderStatus::Submitted) != 0 =>
									stall_countdown = Some(std::time::Instant::now()),
								// IF head is updated AND there are no our transactions:
								// => stop stall countdown timer
								true => stall_countdown = None,
								// IF head is not updated AND stall countdown is not yet completed
								// => do nothing
								false if stall_countdown
									.map(|stall_countdown| std::time::Instant::now() - stall_countdown <
										std::time::Duration::from_millis(STALL_SYNC_TIMEOUT_MS))
									.unwrap_or(true)
									=> (),
								// IF head is not updated AND stall countdown has completed
								// => restart sync
								false => {
									log::info!(
										target: "bridge",
										"Possible Substrate fork detected. Restarting Ethereum headers synchronization.",
									);
									stall_countdown = None;
									eth_sync.restart();
								},
							}
						},
						Err(error) => log::error!(
							target: "bridge",
							"Error retrieving best known header from Substrate node: {:?}",
							error,
						),
					}
				},
				(sub_client, sub_existence_status) = sub_existence_status_future => {
					sub_maybe_client = Some(sub_client);

					match sub_existence_status {
						Ok((sub_header, sub_existence_status)) => eth_sync
							.headers_mut()
							.maybe_orphan_response(&sub_header, sub_existence_status),
						Err(error) => log::error!(
							target: "bridge",
							"Error retrieving existence status from Substrate node: {:?}",
							error,
						),
					}
				},
				(sub_client, sub_submit_header_result) = sub_submit_header_future => {
					sub_maybe_client = Some(sub_client);

					match sub_submit_header_result {
						Ok((_transction_hash, submitted_headers)) => eth_sync
							.headers_mut()
							.headers_submitted(submitted_headers),
						Err(error) => log::error!(
							target: "bridge",
							"Error submitting header to Substrate node: {:?}",
							error,
						),
					}
				},
				(sub_client, sub_receipts_check_result) = sub_receipts_check_future => {
					// we can minimize number of receipts_check calls by checking header
					// logs bloom here, but it may give us false positives (when authorities
					// source is contract, we never need any logs)
					sub_maybe_client = Some(sub_client);

					match sub_receipts_check_result {
						Ok((header, receipts_check_result)) => eth_sync
							.headers_mut()
							.maybe_receipts_response(&header, receipts_check_result),
						Err(error) => log::error!(
							target: "bridge",
							"Error retrieving receipts requirement from Substrate node: {:?}",
							error,
						),
					}
				},
				_ = sub_tick_stream.next() => {
					sub_best_block_required = true;
				},
			}

			// print progress
			progress_context = print_progress(progress_context, &eth_sync);

			// if client is available: wait, or call Substrate RPC methods
			if let Some(sub_client) = sub_maybe_client.take() {
				// the priority is to:
				// 1) get best block - it stops us from downloading/submitting new blocks + we call it rarely;
				// 2) check transactions receipts - it stops us from downloading/submitting new blocks;
				// 3) check existence - it stops us from submitting new blocks;
				// 4) submit header
				
				if sub_best_block_required {
					log::debug!(target: "bridge", "Asking Substrate about best block");
					sub_best_block_future.set(substrate_client::best_ethereum_block(sub_client).fuse());
				} else if let Some(header) = eth_sync.headers().header(EthereumHeaderStatus::MaybeReceipts) {
					log::debug!(
						target: "bridge",
						"Checking if header submission requires receipts: {:?}",
						header.id(),
					);

					let header = header.clone();
					sub_receipts_check_future.set(
						substrate_client::ethereum_receipts_required(sub_client, header).fuse()
					);
				} else if let Some(header) = eth_sync.headers().header(EthereumHeaderStatus::MaybeOrphan) {
					// for MaybeOrphan we actually ask for parent' header existence
					let parent_id = header.parent_id();

					log::debug!(
						target: "bridge",
						"Asking Substrate node for existence of: {:?}",
						parent_id,
					);

					sub_existence_status_future.set(
						substrate_client::ethereum_header_known(sub_client, parent_id).fuse(),
					);
				} else if let Some(headers) = eth_sync.select_headers_to_submit() {
					let ids = match headers.len() {
						1 => format!("{:?}", headers[0].id()),
						2 => format!("[{:?}, {:?}]", headers[0].id(), headers[1].id()),
						len => format!("[{:?} ... {:?}]", headers[0].id(), headers[len - 1].id()),
					};
					log::debug!(
						target: "bridge",
						"Submitting {} header(s) to Substrate node: {:?}",
						headers.len(),
						ids,
					);

					let headers = headers.into_iter().cloned().collect();
					sub_submit_header_future.set(
						substrate_client::submit_ethereum_headers(sub_client, headers).fuse(),
					);

					// remember that we have submitted some headers
					if stall_countdown.is_none() {
						stall_countdown = Some(std::time::Instant::now());
					}
				} else {
					sub_maybe_client = Some(sub_client);
				}
			}

			// if client is available: wait, or call Ethereum RPC methods
			if let Some(eth_client) = eth_maybe_client.take() {
				// the priority is to:
				// 1) get best block - it stops us from downloading new blocks + we call it rarely;
				// 2) check transactions receipts - it stops us from downloading/submitting new blocks;
				// 3) check existence - it stops us from submitting new blocks;
				// 4) submit header

				if eth_best_block_number_required {
					log::debug!(target: "bridge", "Asking Ethereum node about best block number");
					eth_best_block_number_future.set(ethereum_client::best_block_number(eth_client).fuse());
				} else if let Some(header) = eth_sync.headers().header(EthereumHeaderStatus::Receipts) {
					let id = header.id();
					log::debug!(
						target: "bridge",
						"Retrieving receipts for header: {:?}",
						id,
					);
					eth_receipts_future.set(
						ethereum_client::transactions_receipts(
							eth_client,
							id,
							header.header().transactions.clone(),
						).fuse()
					);
				} else if let Some(header) = eth_sync.headers().header(EthereumHeaderStatus::Orphan) {
					// for Orphan we actually ask for parent' header
					let parent_id = header.parent_id();

					log::debug!(
						target: "bridge",
						"Going to download orphan header from Ethereum node: {:?}",
						parent_id,
					);

					eth_orphan_header_future.set(
						ethereum_client::header_by_hash(eth_client, parent_id.1).fuse(),
					);
				} else if let Some(id) = eth_sync.select_new_header_to_download() {
					log::debug!(
						target: "bridge",
						"Going to download new header from Ethereum node: {:?}",
						id,
					);

					eth_new_header_future.set(
						ethereum_client::header_by_number(eth_client, id).fuse(),
					);
				} else {
					eth_maybe_client = Some(eth_client);
				}
			}
		}
	});
}
