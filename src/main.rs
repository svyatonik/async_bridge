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
mod ethereum_sync_loop;
mod ethereum_types;
mod substrate_client;
mod substrate_types;

fn main() {
	initialize();

	ethereum_sync_loop::run(match ethereum_sync_params() {
		Ok(ethereum_sync_params) => ethereum_sync_params,
		Err(err) => {
			log::error!(target: "bridge", "Error parsing parameters: {}", err);
			return;
		},
	});
}

fn initialize() {
	env_logger::init();
}

fn ethereum_sync_params() -> Result<ethereum_sync_loop::EthereumSyncParams, String> {
	let yaml = clap::load_yaml!("cli.yml");
	let matches = clap::App::from_yaml(yaml).get_matches();

	let mut eth_sync_params = ethereum_sync_loop::EthereumSyncParams::default();
	if let Some(eth_host) = matches.value_of("eth-host") {
		eth_sync_params.eth_host = eth_host.into();
	}
	if let Some(eth_port) = matches.value_of("eth-port") {
		eth_sync_params.eth_port = eth_port.parse().map_err(|e| format!("{}", e))?;
	}
	if let Some(sub_host) = matches.value_of("sub-host") {
		eth_sync_params.sub_host = sub_host.into();
	}
	if let Some(sub_port) = matches.value_of("sub-port") {
		eth_sync_params.sub_port = sub_port.parse().map_err(|e| format!("{}", e))?;
	}

	Ok(eth_sync_params)
}
