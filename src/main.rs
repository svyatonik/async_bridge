#![recursion_limit="512"]

mod ethereum_client;
mod ethereum_headers;
mod ethereum_sync;
mod ethereum_types;
mod substrate_client;
mod substrate_types;

use crate::ethereum_types::HeaderStatus as EthereumHeaderStatus;

// TODO: since we only need latest substrate headers (where our transactions are included)
// => better to have subscription here instead of manual polling

// TODO: when substrate reorgs, we need to reset (?) all info that we have received from Substrate


async fn delay(timeout_ms: u64) {
	async_std::task::sleep(std::time::Duration::from_millis(timeout_ms)).await
}

fn interval(timeout_ms: u64) -> impl futures::Stream<Item = ()> {
	futures::stream::unfold((), move |_| async move { delay(timeout_ms).await; Some(((), ())) })
}

async fn exit_future() {
	futures::future::pending().await
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

		let mut eth_maybe_client = None;
		let mut eth_best_block_number_required = false;
		let eth_best_block_number_future = ethereum_client::best_block_number(ethereum_client::client()).fuse();
		let eth_new_header_future = futures::future::Fuse::terminated();
		let eth_orphan_header_future = futures::future::Fuse::terminated();
		let eth_receipts_future = futures::future::Fuse::terminated();
		let eth_tick_stream = interval(10 * 1000).fuse();

		let mut sub_maybe_client = None;
		let mut sub_best_block_required = false;
		let sub_best_block_future = substrate_client::best_ethereum_block(substrate_client::client()).fuse();
		let sub_receipts_check_future = futures::future::Fuse::terminated();
		let sub_existence_status_future = futures::future::Fuse::terminated();
		let sub_submit_header_future = futures::future::Fuse::terminated();
		let sub_tick_stream = interval(1000).fuse();

		let exit_future = exit_future().fuse();

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
			sub_tick_stream,
			exit_future
		);

		loop {
			futures::select! {
				(eth_client, eth_best_block_number) = eth_best_block_number_future => {
					eth_maybe_client = Some(eth_client);
					eth_best_block_number_required = false;

					match eth_best_block_number {
						Ok(eth_best_block_number) => eth_sync.ethereum_best_header_number_response(eth_best_block_number),
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
						Ok(sub_best_block) => eth_sync.substrate_best_header_response(sub_best_block),
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
						Ok((sub_header, sub_existence_status)) => eth_sync.headers_mut().maybe_orphan_response(&sub_header, sub_existence_status),
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
						Ok((_transction_hash, submitted_headers)) => eth_sync.headers_mut().headers_submitted(submitted_headers),
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
						Ok((header, receipts_check_result)) => eth_sync.headers_mut().maybe_receipts_response(&header, receipts_check_result),
						Err(error) => log::error!(
							target: "bridge",
							"Error retrieving receipts requirement from Substrate node: {:?}",
							error,
						),
					}
				},
				_ = sub_tick_stream.next() => {
					if eth_sync.is_header_submitted_recently() {
						sub_best_block_required = true;
					}
				},
				_ = exit_future => {
					println!("stopping (exit future signalled)");
					break;
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
				} else if let Some(header_for_existence_status) = eth_sync.headers().header(EthereumHeaderStatus::MaybeOrphan) {
					// for MaybeOrphan we actually ask for parent' header existence
					let parent_id = header_for_existence_status.parent_id();

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
				} else if let Some(header_for_receipts_retrieval) = eth_sync.headers().header(EthereumHeaderStatus::Receipts) {
					log::debug!(
						target: "bridge",
						"Retrieving receipts for header: {:?}",
						header_for_receipts_retrieval,
					);
					eth_receipts_future.set(
						ethereum_client::transactions_receipts(eth_client, header_for_receipts_retrieval.id(), header_for_receipts_retrieval.header().transactions.clone()).fuse()
					);
				} else if let Some(orphan_header_to_download) = eth_sync.headers().header(EthereumHeaderStatus::Orphan) {
					// for Orphan we actually ask for parent' header
					let parent_id = orphan_header_to_download.parent_id();

					log::debug!(
						target: "bridge",
						"Going to download orphan header from Ethereum node: {:?}",
						parent_id,
					);

					eth_orphan_header_future.set(
						ethereum_client::header_by_hash(eth_client, parent_id.1).fuse(),
					);
				} else if let Some(new_header_to_download) = eth_sync.select_new_header_to_download() {
					log::debug!(
						target: "bridge",
						"Going to download new header from Ethereum node: {:?}",
						new_header_to_download,
					);

					eth_new_header_future.set(
						ethereum_client::header_by_number(eth_client, new_header_to_download).fuse(),
					);
				} else {
					eth_maybe_client = Some(eth_client);
				}
			}
		}
	});
}
