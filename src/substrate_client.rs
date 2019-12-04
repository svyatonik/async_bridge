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
	let header = into_substrate_ethereum_header(header.extract().0);
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
pub async fn submit_ethereum_header(
	client: Client,
	header: QueuedEthereumHeader,
) -> (Client, Result<(TransactionHash, EthereumHeaderId), Error>) {
	let id = header.id();
	let (header, receipts) = header.extract();
	let call = node_runtime::Call::BridgeEthPoa(
		node_runtime::BridgeEthPoaCall::import_header(
			into_substrate_ethereum_header(header),
			into_substrate_ethereum_receipts(receipts),
		),
	);
	let transaction = node_runtime::UncheckedExtrinsic {
		signature: None,
		function: call,
	};
	let encoded_transaction = transaction.encode();
	let (client, transaction_hash) = call_rpc(
		client,
		"author_submitExtrinsic",
		Params::Array(vec![
			to_value(Bytes(encoded_transaction)).unwrap(),
		]),
	).await;
	(client, transaction_hash.map(|transaction_hash| (transaction_hash, id)))
}

/// Calls RPC on Substrate node.
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

/*
use std::collections::HashMap;
use crate::ethereum_types::{
	HeaderId as EthereumHeaderId,
	Header as EthereumHeader,
	H256,
	Receipt as EthereumReceipt,
};

/// Substrate client type.
pub struct Client {
	/// Best known Ethereum block.
	best_ethereum_block: EthereumHeaderId,
	/// All stored headers.
	ethereum_headers: HashMap<H256, (EthereumHeader, Vec<EthereumReceipt>)>,
}

/// All possible errors that can occur during interacting with Substrate node.
#[derive(Debug)]
pub enum Error { }

/// Returns client that is able to call RPCs on Substrate node.
pub fn client() -> Client {
	Client {
		best_ethereum_block: EthereumHeaderId(0, "a3c565fc15c7478862d50ccd6561e3c06b24cc509bf388941c25ea985ce32cb9".parse().unwrap()),
		ethereum_headers: std::collections::HashMap::new(),
	}
}

/// Returns best Ethereum block that Substrate runtime knows of.
pub async fn best_ethereum_block(client: Client) -> (Client, Result<EthereumHeaderId, Error>) {
	let best_ethereum_block = client.best_ethereum_block.clone();
	(client, Ok(best_ethereum_block))
}

/// Returns true if transactions receipts are required for Ethereum header submission.
pub async fn ethereum_receipts_required(
	client: Client,
	id: EthereumHeaderId,
) -> (Client, Result<(EthereumHeaderId, bool), Error>) {
	(client, Ok((id, true)))
}

/// Returns true if Ethereum header is known to Substrate runtime.
pub async fn ethereum_header_known(
	client: Client,
	id: EthereumHeaderId,
) -> (Client, Result<(EthereumHeaderId, bool), Error>) {
	let is_known_header = client.ethereum_headers.contains_key(&id.1);
	(client, Ok((id, is_known_header)))
}

/// Submits Ethereum header to Substrate runtime.
pub async fn submit_ethereum_header(
	mut client: Client,
	header: EthereumHeader,
	receipts: Option<Vec<EthereumReceipt>>,
) -> (Client, Result<EthereumHeaderId, Error>) {
	let id = (&header).into();
	client.best_ethereum_block = id;
	client.ethereum_headers.insert(id.1, (header, receipts.unwrap()));
	(client, Ok(id))
}
*/