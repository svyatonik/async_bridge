use crate::ethereum_types::{HeaderId, HeaderStatus, QueuedHeader};
use crate::ethereum_headers::QueuedHeaders;

/// Ethereum synchronization parameters.
#[derive(Debug)]
pub struct HeadersSyncParams {
	/// Maximal number of ethereum headers to pre-download.
	pub max_future_headers_to_download: usize,
	/// Maximal number of active (we believe) submit header transactions.
	pub max_headers_in_submitted_status: usize,
}

impl Default for HeadersSyncParams {
	fn default() -> Self {
		HeadersSyncParams {
			max_future_headers_to_download: 128,
			max_headers_in_submitted_status: 8,
		}
	}
}

/// Ethereum headers synchronization context.
#[derive(Debug, Default)]
pub struct HeadersSync {
	/// Synchronization parameters.
	params: HeadersSyncParams,
	/// Best header number known to Ethereum node.
	target_header_number: Option<u64>,
	/// Best header known to Substrate node.
	best_header: Option<HeaderId>,
	/// The number of header that we have submitted recently.
	unconfirmed_best_header_number: u64,
	/// Headers queue.
	headers: QueuedHeaders,
}

impl HeadersSync {
	/// Returns true if we have submitted header recently.
	pub fn is_header_submitted_recently(&self) -> bool {
		// TODO: this method should be removed after subscription to Substrate headers.
		true
	}

	/// Returns true if we have synced almost all known headers.
	pub fn is_almost_synced(&self) -> bool {
		self.best_header
			.and_then(|best| self.target_header_number.map(|target| (best, target)))
			.map(|(best, target)| target.saturating_sub(best.0) < 4)
			.unwrap_or(false)
	}

	/// Returns reference to the headers queue.
	pub fn headers(&self) -> &QueuedHeaders {
		&self.headers
	}

	/// Returns mutable reference to the headers queue.
	pub fn headers_mut(&mut self) -> &mut QueuedHeaders {
		&mut self.headers
	}

	/// Select header that needs to be downloaded from the Ethereum node.
	pub fn select_new_header_to_download(&self) -> Option<u64> {
		// if we haven't received best header from Ethereum node yet, there's nothing we can download
		let target_header_number = self.target_header_number.clone()?;

		// if we haven't received known best header from Substrate node yet, there's nothing we can download
		let best_header = self.best_header.as_ref()?;

		// if there's too many headers in the queue, stop downloading
		let in_memory_headers = self.headers.total_headers();
		if in_memory_headers >= self.params.max_future_headers_to_download {
			return None;
		}

		// we assume that there were no reorgs if we have already downloaded best header
		let best_downloaded_number = std::cmp::max(
			self.headers.best_queued_number(),
			best_header.0,
		);
		if best_downloaded_number == target_header_number {
			return None;
		}

		// download new header
		Some(best_downloaded_number + 1)
	}

	/// Select header that needs to be submitted to the Substrate node.
	pub fn select_header_to_submit(&self) -> Option<&QueuedHeader> {
		if self.headers.headers_in_submit_status() >= self.params.max_headers_in_submitted_status {
			return None;
		}

		// TODO: submitting known (to Substrate) headers should not be penalized

		// TODO: if there's long fork, the sync may stall, because we won't know if
		// the side header has been accepted until reorg happens
		// => there should be another mechanism to handle this
		self.headers.header(HeaderStatus::Ready)
	}

	/// Receive new target header number from the Ethereum node.
	pub fn ethereum_best_header_number_response(&mut self, best_header_number: u64) {
		log::debug!(target: "bridge", "Received best header number from Ethereum: {}", best_header_number);
		self.target_header_number = Some(best_header_number);
	}

	/// Receive new best header from the Substrate node.
	pub fn substrate_best_header_response(&mut self, best_header: HeaderId) {
		log::debug!(target: "bridge", "Received best known header from Substrate: {:?}", best_header);

		// early return if it is still the same
		if self.best_header == Some(best_header) {
			return;
		}

		// remember that this header is now known to the Substrate runtime
		self.headers.substrate_best_header_response(&best_header);

		// finally remember the best header itself
		self.best_header = Some(best_header);
	}

	/// Synchronization tick (called periodically).
	pub fn tick(&mut self) {
	}
}

#[cfg(test)]
mod tests {
	use crate::ethereum_headers::tests::{header, id};
	use crate::ethereum_types::{H256, HeaderStatus};
	use super::*;

	fn side_hash(number: u64) -> H256 {
		H256::from_low_u64_le(1000 + number)
	}

	#[test]
	fn select_new_header_to_download_works() {
		let mut eth_sync = HeadersSync::default();

		// both best && target headers are unknown
		assert_eq!(eth_sync.select_new_header_to_download(), None);

		// best header is known, target header is unknown
		eth_sync.best_header = Some(HeaderId(0, Default::default()));
		assert_eq!(eth_sync.select_new_header_to_download(), None);

		// target header is known, best header is unknown
		eth_sync.best_header = None;
		eth_sync.target_header_number = Some(100);
		assert_eq!(eth_sync.select_new_header_to_download(), None);

		// when our best block has the same number as the target
		eth_sync.best_header = Some(HeaderId(100, Default::default()));
		assert_eq!(eth_sync.select_new_header_to_download(), None);

		// when we actually need a new header
		eth_sync.target_header_number = Some(101);
		assert_eq!(eth_sync.select_new_header_to_download(), Some(101));

		// when there are too many headers scheduled for submitting
		for i in 1..1000 {
			eth_sync.headers.header_response(header(i).header().clone());
		}
		assert_eq!(eth_sync.select_new_header_to_download(), None);
	}

	#[test]
	fn sync_without_reorgs_works() {
		let mut eth_sync = HeadersSync::default();
		eth_sync.params.max_headers_in_submitted_status = 1;

		// ethereum reports best header #102
		eth_sync.ethereum_best_header_number_response(102);

		// substrate reports that it is at block #100
		eth_sync.substrate_best_header_response(id(100));

		// block #101 is downloaded first
		assert_eq!(eth_sync.select_new_header_to_download(), Some(101));
		eth_sync.headers.header_response(header(101).header().clone());

		// now header #101 is ready to be submitted
		assert_eq!(eth_sync.headers.header(HeaderStatus::MaybeReceipts), Some(&header(101)));
		eth_sync.headers.maybe_receipts_response(&id(101), false);
		assert_eq!(eth_sync.headers.header(HeaderStatus::Ready), Some(&header(101)));
		assert_eq!(eth_sync.select_header_to_submit(), Some(&header(101)));

		// and header #102 is ready to be downloaded
		assert_eq!(eth_sync.select_new_header_to_download(), Some(102));
		eth_sync.headers.header_response(header(102).header().clone());

		// receive submission confirmation
		eth_sync.headers.header_submitted(&id(101));

		// we have nothing to submit because previous header hasn't been confirmed yet
		// (and we allow max 1 submit transaction in the wild)
		assert_eq!(eth_sync.headers.header(HeaderStatus::MaybeReceipts), Some(&header(102)));
		eth_sync.headers.maybe_receipts_response(&id(102), false);
		assert_eq!(eth_sync.headers.header(HeaderStatus::Ready), Some(&header(102)));
		assert_eq!(eth_sync.select_header_to_submit(), None);

		// substrate reports that it has imported block #101
		eth_sync.substrate_best_header_response(id(101));

		// and we are ready to submit #102
		assert_eq!(eth_sync.select_header_to_submit(), Some(&header(102)));
		eth_sync.headers.header_submitted(&id(102));

		// substrate reports that it has imported block #102
		eth_sync.substrate_best_header_response(id(102));

		// and we have nothing to download
		assert_eq!(eth_sync.select_new_header_to_download(), None);
	}

	#[test]
	fn sync_with_orphan_headers_work() {
		let mut eth_sync = HeadersSync::default();

		// ethereum reports best header #102
		eth_sync.ethereum_best_header_number_response(102);

		// substrate reports that it is at block #100, but it isn't part of best chain
		eth_sync.substrate_best_header_response(HeaderId(100, side_hash(100)));

		// block #101 is downloaded first
		assert_eq!(eth_sync.select_new_header_to_download(), Some(101));
		eth_sync.headers.header_response(header(101).header().clone());

		// we can't submit header #101, because its parent status is unknown
		assert_eq!(eth_sync.select_header_to_submit(), None);

		// instead we are trying to determine status of its parent (#100)
		assert_eq!(eth_sync.headers.header(HeaderStatus::MaybeOrphan), Some(&header(101)));

		// and the status is still unknown
		eth_sync.headers.maybe_orphan_response(&id(100), false);

		// so we consider #101 orphaned now && will download its parent - #100
		assert_eq!(eth_sync.headers.header(HeaderStatus::Orphan), Some(&header(101)));
		eth_sync.headers.header_response(header(100).header().clone());

		// we can't submit header #100, because its parent status is unknown
		assert_eq!(eth_sync.select_header_to_submit(), None);

		// instead we are trying to determine status of its parent (#99)
		assert_eq!(eth_sync.headers.header(HeaderStatus::MaybeOrphan), Some(&header(100)));

		// and the status is known, so we move previously orphaned #100 and #101 to ready queue
		eth_sync.headers.maybe_orphan_response(&id(99), true);

		// and we are ready to submit #100
		assert_eq!(eth_sync.headers.header(HeaderStatus::MaybeReceipts), Some(&header(100)));
		eth_sync.headers.maybe_receipts_response(&id(100), false);
		assert_eq!(eth_sync.select_header_to_submit(), Some(&header(100)));
		eth_sync.headers.header_submitted(&id(100));

		// and we are ready to submit #101
		assert_eq!(eth_sync.headers.header(HeaderStatus::MaybeReceipts), Some(&header(101)));
		eth_sync.headers.maybe_receipts_response(&id(101), false);
		assert_eq!(eth_sync.select_header_to_submit(), Some(&header(101)));
		eth_sync.headers.header_submitted(&id(101));
	}
}
