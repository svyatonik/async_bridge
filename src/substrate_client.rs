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
