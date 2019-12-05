use codec::{Encode, Decode};
use jsonrpsee_core::{client::ClientError, common::Params};
use jsonrpsee_http::{HttpClient, RequestError, http_client};
use serde_json::{from_value, to_value};
use crate::ethereum_types::{
	Bytes,
	H256,
	HeaderId as EthereumHeaderId,
	QueuedHeader as QueuedEthereumHeader,
};
use crate::substrate_types::{
	TransactionHash,
	into_substrate_ethereum_header,
	into_substrate_ethereum_receipts,
};

/// Ethereum client type.
pub type Client = HttpClient;

/// All possible errors that can occur during interacting with Ethereum node.
#[derive(Debug)]
pub enum Error {
	/// Request start failed.
	StartRequestFailed(RequestError),
	/// Request not found (should never occur?).
	RequestNotFound,
	/// Failed to receive response.
	ResponseRetrievalFailed(ClientError<RequestError>),
	/// Failed to parse response.
	ResponseParseFailed,
}

/// Returns client that is able to call RPCs on Substrate node.
pub fn client() -> Client {
	http_client("http://127.0.0.1:11010")
}

/// Returns best Ethereum block that Substrate runtime knows of.
pub async fn best_ethereum_block(client: Client) -> (Client, Result<EthereumHeaderId, Error>) {
	let (client, result) = call_rpc::<(u64, H256)>(
		client,
		"state_call",
		Params::Array(vec![
			to_value("EthereumHeadersApi_best_block").unwrap(),
			to_value("0x").unwrap(),
		]),
	).await;
	(client, result.map(|(num, hash)| EthereumHeaderId(num, hash)))
}

/// Returns true if transactions receipts are required for Ethereum header submission.
pub async fn ethereum_receipts_required(
	client: Client,
	header: QueuedEthereumHeader,
) -> (Client, Result<(EthereumHeaderId, bool), Error>) {
	let id = header.id();
	let header = into_substrate_ethereum_header(header.header());
	let encoded_header = header.encode();
	let (client, receipts_required) = call_rpc(
		client,
		"state_call",
		Params::Array(vec![
			to_value("EthereumHeadersApi_is_import_requires_receipts").unwrap(),
			to_value(Bytes(encoded_header)).unwrap(),
		]),
	).await;
	(client, receipts_required.map(|receipts_required| (id, receipts_required)))
}

/// Returns true if Ethereum header is known to Substrate runtime.
pub async fn ethereum_header_known(
	client: Client,
	id: EthereumHeaderId,
) -> (Client, Result<(EthereumHeaderId, bool), Error>) {
	// TODO: it is unknown if pruned => we need to cut off old queries
	let encoded_id = id.1.encode();
	let (client, is_known_block) = call_rpc(
		client,
		"state_call",
		Params::Array(vec![
			to_value("EthereumHeadersApi_is_known_block").unwrap(),
			to_value(Bytes(encoded_id)).unwrap(),
		]),
	).await;
	(client, is_known_block.map(|is_known_block| (id, is_known_block)))
}

/// Submits Ethereum header to Substrate runtime.
pub async fn submit_ethereum_headers(
	client: Client,
	headers: Vec<QueuedEthereumHeader>,
) -> (Client, Result<(TransactionHash, Vec<EthereumHeaderId>), Error>) {
	let ids = headers.iter().map(|header| header.id()).collect();
	let (client, genesis_hash) = block_hash_by_number(client, 0).await;
	let genesis_hash = match genesis_hash {
		Ok(genesis_hash) => genesis_hash,
		Err(err) => return (client, Err(err)),
	};
	let (client, nonce) = next_account_index(client, sp_keyring::AccountKeyring::Alice.to_account_id()).await;
	let nonce = match nonce {
		Ok(nonce) => nonce,
		Err(err) => return (client, Err(err)),
	};
	let transaction = create_submit_transaction(
		headers,
		sp_keyring::AccountKeyring::Alice,
		nonce,
		genesis_hash,
	);
	let encoded_transaction = transaction.encode();
	let (client, transaction_hash) = call_rpc(
		client,
		"author_submitExtrinsic",
		Params::Array(vec![
			to_value(Bytes(encoded_transaction)).unwrap(),
		]),
	).await;
	(client, transaction_hash.map(|transaction_hash| (transaction_hash, ids)))
}

/// Get Substrate block hash by its number.
async fn block_hash_by_number(client: Client, number: u64) -> (Client, Result<H256, Error>) {
	call_rpc(
		client,
		"chain_getBlockHash",
		Params::Array(vec![
			to_value(number).unwrap(),
		]),
	).await
}

/// Get substrate account nonce.
async fn next_account_index(
	client: Client,
	account: node_primitives::AccountId,
) -> (Client, Result<node_primitives::Index, Error>) {
	use sp_core::crypto::Ss58Codec;

	let (client, index) = call_rpc_u64(
		client,
		"system_accountNextIndex",
		Params::Array(vec![
			to_value(account.to_ss58check()).unwrap(),
		]),
	).await;
	(client, index.map(|index| index as _))
}

/// Calls RPC on Substrate node that returns Bytes.
async fn call_rpc<T: Decode>(
	mut client: Client,
	method: &'static str,
	params: Params,
) -> (Client, Result<T, Error>) {
	async fn do_call_rpc<T: Decode>(
		client: &mut Client,
		method: &'static str,
		params: Params,
	) -> Result<T, Error> {
		let request_id = client
			.start_request(method, params)
			.await
			.map_err(Error::StartRequestFailed)?;
		// WARN: if there'll be need for executing >1 request at a time, we should avoid
		// calling request_by_id
		let response = client
			.request_by_id(request_id)
			.ok_or(Error::RequestNotFound)?
			.await
			.map_err(Error::ResponseRetrievalFailed)?;
		let encoded_response: Bytes = from_value(response).map_err(|_| Error::ResponseParseFailed)?;
		Decode::decode(&mut &encoded_response.0[..]).map_err(|_| Error::ResponseParseFailed)
	}

	let result = do_call_rpc(&mut client, method, params).await;
	(client, result)
}

/// Calls RPC on Substrate node that returns u64.
async fn call_rpc_u64(
	mut client: Client,
	method: &'static str,
	params: Params,
) -> (Client, Result<u64, Error>) {
	async fn do_call_rpc(
		client: &mut Client,
		method: &'static str,
		params: Params,
	) -> Result<u64, Error> {
		let request_id = client
			.start_request(method, params)
			.await
			.map_err(Error::StartRequestFailed)?;
		// WARN: if there'll be need for executing >1 request at a time, we should avoid
		// calling request_by_id
		let response = client
			.request_by_id(request_id)
			.ok_or(Error::RequestNotFound)?
			.await
			.map_err(Error::ResponseRetrievalFailed)?;
		response.as_u64().ok_or(Error::ResponseParseFailed)
	}

	let result = do_call_rpc(&mut client, method, params).await;
	(client, result)
}

/// Create Substrate transaction for submitting Ethereum header.
fn create_submit_transaction(
	headers: Vec<QueuedEthereumHeader>,
	account: sp_keyring::AccountKeyring,
	index: node_primitives::Index,
	genesis_hash: H256,
) -> node_runtime::UncheckedExtrinsic {
	use sp_core::crypto::Pair;
	use sp_runtime::traits::IdentifyAccount;

	let signer = account.pair();

	let function = node_runtime::Call::BridgeEthPoa(
		node_runtime::BridgeEthPoaCall::import_headers(
			headers
				.into_iter()
				.map(|header| {
					let (header, receipts) = header.extract();
					(
						into_substrate_ethereum_header(&header),
						into_substrate_ethereum_receipts(&receipts),
					)
				})
				.collect(),
		),
	);

	let extra = |i: node_primitives::Index, f: node_primitives::Balance| {
		(
			frame_system::CheckVersion::<node_runtime::Runtime>::new(),
			frame_system::CheckGenesis::<node_runtime::Runtime>::new(),
			frame_system::CheckEra::<node_runtime::Runtime>::from(sp_runtime::generic::Era::Immortal),
			frame_system::CheckNonce::<node_runtime::Runtime>::from(i),
			frame_system::CheckWeight::<node_runtime::Runtime>::new(),
			pallet_transaction_payment::ChargeTransactionPayment::<node_runtime::Runtime>::from(f),
			Default::default(),
		)
	};
	let raw_payload = node_runtime::SignedPayload::from_raw(
		function,
		extra(index, 0),
		(
			198, // VERSION.spec_version as u32,
			genesis_hash,
			genesis_hash,
			(),
			(),
			(),
			(),
		),
	);
	let signature = raw_payload.using_encoded(|payload| signer.sign(payload));
	let signer: sp_runtime::MultiSigner = signer.public().into();
	let (function, extra, _) = raw_payload.deconstruct();

	node_runtime::UncheckedExtrinsic::new_signed(
		function,
		signer.into_account().into(),
		signature.into(),
		extra,
	)
}
