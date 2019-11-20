pub use web3::types::{H256, U128, U64};

/// When header is just received from the Ethereum node, we check that it has
/// both number and hash fields filled.
pub const HEADER_ID_PROOF: &'static str = "checked on retrieval; qed";

/// Ethereum header type.
pub type Header = web3::types::Block<()>;

/// Ethereum transaction receipt type.
pub type Receipt = web3::types::TransactionReceipt;

/// Ethereum transaction type.
pub type Transaction = web3::types::Transaction;

/// Ethereum header Id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeaderId(pub u64, pub H256);

impl From<&Header> for HeaderId {
	fn from(header: &Header) -> HeaderId {
		HeaderId(header.number.expect(HEADER_ID_PROOF).as_u64(), header.hash.expect(HEADER_ID_PROOF))
	}
}

/// Ethereum header synchronization status.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HeaderStatus {
	/// Header is unknown.
	Unknown,
	/// Header is in MaybeOrphan queue.
	MaybeOrphan,
	/// Header is in Orphan queue.
	Orphan,
	/// Header is in MaybeReceipts queue.
	MaybeReceipts,
	/// Header is in Receipts queue.
	Receipts,
	/// Header is in Ready queue.
	Ready,
	/// Header has been recently submitted to the Substrate runtime.
	Submitted,
	/// Header is known to the Substrate runtime.
	Synced,
}

#[derive(Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
pub struct QueuedHeader {
	header: Header,
	receipts: Option<Vec<Receipt>>,
}

impl QueuedHeader {
	/// Creates new queued header.
	pub fn new(header: Header) -> Self {
		QueuedHeader { header, receipts: None }
	}

	/// Returns ID of header.
	pub fn id(&self) -> HeaderId {
		(&self.header).into()
	}

	/// Returns ID of parent header.
	pub fn parent_id(&self) -> HeaderId {
		HeaderId(
			self.header.number.expect(HEADER_ID_PROOF).as_u64() - 1,
			self.header.parent_hash,
		)
	}

	/// Returns reference to header.
	pub fn header(&self) -> &Header {
		&self.header
	}

	/// Returns reference to associated transactions receipts.
	pub fn receipts(&self) -> &Option<Vec<Receipt>> {
		&self.receipts
	}

	/// Set associated transaction receipts.
	pub fn set_receipts(mut self, receipts: Vec<Receipt>) -> Self {
		self.receipts = Some(receipts);
		self
	}
}
