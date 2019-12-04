use jsonrpsee_core::{client::ClientError, common::Params};
use jsonrpsee_http::{HttpClient, RequestError, http_client};
use serde::de::DeserializeOwned;
use serde_json::{from_value, to_value};
use crate::ethereum_types::{H256, Header, HeaderId, Receipt, Transaction, U64, U128};

/// Proof of hash serialization success.
const HASH_SERIALIZATION_PROOF: &'static str = "hash serialization never fails; qed";
/// Proof of integer serialization success.
const INT_SERIALIZATION_PROOF: &'static str = "integer serialization never fails; qed";
/// Proof of bool serialization success.
const BOOL_SERIALIZATION_PROOF: &'static str = "bool serialization never fails; qed";

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
	ResponseParseFailed(serde_json::Error),
	/// We have received header with missing number and hash fields.
	IncompleteHeader,
	/// We have received receipt with missing gas_used field.
	IncompleteReceipt,
}

/// Returns client that is able to call RPCs on Ethereum node.
pub fn client() -> Client {
	http_client("http://127.0.0.1:8545")
}

/// Retrieve best known block number from Ethereum node.
pub async fn best_block_number(client: Client) -> (Client, Result<u64, Error>) {
	let (client, result) = call_rpc::<U64>(
		client,
		"eth_blockNumber",
		Params::None,
	).await;
	(client, result.map(|x| x.as_u64()))
}

/// Retrieve block header by its number from Ethereum node.
pub async fn header_by_number(client: Client, number: u64) -> (Client, Result<Header, Error>) {
	let (client, header) = call_rpc(
		client,
		"eth_getBlockByNumber",
		Params::Array(vec![
			to_value(U64::from(number)).expect(INT_SERIALIZATION_PROOF),
			to_value(false).expect(BOOL_SERIALIZATION_PROOF),
		]),
	).await;
	(client, header.and_then(|header: Header| match header.number.is_none() && header.hash.is_none() {
		true => Ok(header),
		false => Err(Error::IncompleteHeader),
	}))
}

/// Retrieve block header by its hash from Ethereum node.
pub async fn header_by_hash(client: Client, hash: H256) -> (Client, Result<Header, Error>) {
	let (client, header) = call_rpc(
		client,
		"eth_getBlockByHash",
		Params::Array(vec![
			to_value(hash).expect(HASH_SERIALIZATION_PROOF),
			to_value(false).expect(BOOL_SERIALIZATION_PROOF),
		]),
	).await;
	(client, header.and_then(|header: Header| match header.number.is_none() && header.hash.is_none() {
		true => Ok(header),
		false => Err(Error::IncompleteHeader),
	}))
}

/// Retrieve transactions receipts for given block.
pub async fn transactions_receipts(
	client: Client,
	id: HeaderId,
) -> (Client, Result<(HeaderId, Vec<Receipt>), Error>) {
	let (mut client, transactions_count) = block_transactions_count(client, id.1).await;
	let transactions_count =  match transactions_count {
		Ok(transactions_count) => transactions_count,
		Err(error) => return (client, Err(error)),
	};
	let mut transactions_hashes = Vec::with_capacity(transactions_count as usize);
	for i in 0..transactions_count {
		let (next_client, transaction) = block_transaction(client, id.1, i).await;
		let transaction = match transaction {
			Ok(transaction) => transaction,
			Err(error) => return (next_client, Err(error)),
		};
		transactions_hashes.push(transaction.hash);
		client = next_client;
	}
	let mut transactions_receipts = Vec::with_capacity(transactions_hashes.len());
	for transactions_hash in transactions_hashes {
		let (next_client, transaction_receipt) = transaction_receipt(client, transactions_hash).await;
		let transaction_receipt = match transaction_receipt {
			Ok(transaction_receipt) => transaction_receipt,
			Err(error) => return (next_client, Err(error)),
		};
		transactions_receipts.push(transaction_receipt);
		client = next_client;
	}
	(client, Ok((id, transactions_receipts)))
}

/// Retrieve number of transactions in a block by its hash.
async fn block_transactions_count(client: Client, hash: H256) -> (Client, Result<u64, Error>) {
	let (client, result) = call_rpc::<U128>(
		client,
		"eth_getBlockTransactionCountByHash",
		Params::Array(vec![
			to_value(hash).expect(HASH_SERIALIZATION_PROOF),
		]),
	).await;
	(client, result.map(|x| x.as_u64()))
}

/// Retrieve transaction by block hash and transaction index.
async fn block_transaction(
	client: Client,
	hash: H256,
	index: u64,
) -> (Client, Result<Transaction, Error>) {
	call_rpc(
		client,
		"eth_getTransactionByBlockHashAndIndex",
		Params::Array(vec![
			to_value(hash).expect(HASH_SERIALIZATION_PROOF),
			to_value(U128::from(index)).expect(INT_SERIALIZATION_PROOF),
		]),
	).await
}

/// Retrieve transaction receipt by transaction hash.
async fn transaction_receipt(client: Client, hash: H256) -> (Client, Result<Receipt, Error>) {
	let (client, receipt) = call_rpc::<Receipt>(
		client,
		"eth_getTransactionReceipt",
		Params::Array(vec![
			to_value(hash).expect(HASH_SERIALIZATION_PROOF),
		]),
	).await;
	(client, receipt.and_then(|receipt| {
		match receipt.gas_used.is_some() {
			true => Ok(receipt),
			false => Err(Error::IncompleteReceipt),
		}
	}))
}

/// Calls RPC on Ethereum node.
async fn call_rpc<T: DeserializeOwned>(
	mut client: Client,
	method: &'static str,
	params: Params,
) -> (Client, Result<T, Error>) {
	async fn do_call_rpc<T: DeserializeOwned>(
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
		from_value(response).map_err(Error::ResponseParseFailed)
	}

	let result = do_call_rpc(&mut client, method, params).await;
	(client, result)
}
