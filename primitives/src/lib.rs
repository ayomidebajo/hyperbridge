// Copyright (C) Polytope Labs Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Traits and types required to compose the tesseract relayer
pub mod config;
#[cfg(feature = "testing")]
pub mod mocks;
pub mod queue;

use futures::Stream;
pub use ismp::events::StateMachineUpdated;
use ismp::{
	consensus::{ConsensusStateId, StateCommitment, StateMachineHeight, StateMachineId},
	events::Event,
	host::StateMachine,
	messaging::{ConsensusMessage, CreateConsensusState, Message},
	router::Post,
	util::Keccak256,
};
pub use pallet_ismp_relayer::withdrawal::{Signature, WithdrawalProof};
use primitive_types::{H256, U256};
use sp_core::keccak_256;
use std::{
	fmt::{Debug, Display, Formatter},
	ops::{Add, Mul},
	pin::Pin,
	sync::Arc,
	time::Duration,
};

/// Ideal Currency unit denominated in 18 decimals
#[derive(Copy, Clone, Default, Ord, PartialOrd, Eq, PartialEq)]
pub struct Cost(pub U256);

impl Mul<U256> for Cost {
	type Output = Cost;

	fn mul(self, rhs: U256) -> Self::Output {
		Cost(self.0 * rhs)
	}
}

impl Add<Cost> for Cost {
	type Output = Self;

	fn add(self, rhs: Cost) -> Self::Output {
		Cost(self.0 + rhs.0)
	}
}

impl Add<U256> for Cost {
	type Output = Self;

	fn add(self, rhs: U256) -> Self::Output {
		Cost(self.0 + rhs)
	}
}

impl Cost {
	pub fn display(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		let val_as_str = self.0.to_string();
		let mut characters = val_as_str.chars().collect::<Vec<_>>();
		// pad with zeros if length is less than 18
		if characters.len() <= 18 {
			let rem = 18 - characters.len();
			(0..=rem).into_iter().for_each(|_| characters.insert(0, '0'));
		}
		// Insert decimal point
		let pointer = characters.len().saturating_sub(18);
		characters.insert(pointer, '.');
		let value = characters.into_iter().collect::<String>();
		f.write_str(&value)
	}
}

impl Debug for Cost {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		self.display(f)
	}
}

impl Display for Cost {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		self.display(f)
	}
}

impl From<U256> for Cost {
	fn from(value: U256) -> Self {
		Cost(value)
	}
}

#[derive(Copy, Clone, Debug, Default)]
pub struct EstimateGasReturnParams {
	pub execution_cost: Cost,
	pub successful_execution: bool,
}

/// Provides an interface for accessing new events and ISMP data on the chain which must be
/// relayed to the counterparty chain.

#[derive(Copy, Clone, Debug)]
pub struct Query {
	pub source_chain: StateMachine,
	pub dest_chain: StateMachine,
	pub nonce: u64,
	pub commitment: H256,
}

/// A type tha should be returned when messages are submitted successfully
pub enum TxReceipt {
	/// Request variant
	Request { query: Query, height: u64 },
	/// Response variant
	Response { query: Query, request_commitment: H256, height: u64 },
}

/// Stream alias
pub type BoxStream<I> = Pin<Box<dyn Stream<Item = Result<I, anyhow::Error>> + Send>>;

pub struct Hasher;

impl Keccak256 for Hasher {
	fn keccak256(bytes: &[u8]) -> H256 {
		keccak_256(bytes).into()
	}
}

#[async_trait::async_trait]
pub trait IsmpProvider: Send + Sync {
	/// Query the latest consensus state of a client
	async fn query_consensus_state(
		&self,
		at: Option<u64>,
		id: ConsensusStateId,
	) -> Result<Vec<u8>, anyhow::Error>;

	/// Query the latest height at which some state machine was last updated
	async fn query_latest_height(&self, id: StateMachineId) -> Result<u32, anyhow::Error>;

	/// Query the State machine commitment at the provided height
	async fn query_state_machine_commitment(
		&self,
		height: StateMachineHeight,
	) -> Result<StateCommitment, anyhow::Error>;

	/// Query the timestamp at which the client was last updated
	async fn query_state_machine_update_time(
		&self,
		height: StateMachineHeight,
	) -> Result<Duration, anyhow::Error>;

	/// Query the challenge period for client
	async fn query_challenge_period(&self, id: ConsensusStateId)
		-> Result<Duration, anyhow::Error>;

	/// Query the latest timestamp for chain
	async fn query_timestamp(&self) -> Result<Duration, anyhow::Error>;

	/// Query a requests proof
	/// Return the scale encoded proof
	async fn query_requests_proof(
		&self,
		at: u64,
		keys: Vec<Query>,
	) -> Result<Vec<u8>, anyhow::Error>;

	/// Query a responses proof
	/// Return the scale encoded proof
	async fn query_responses_proof(
		&self,
		at: u64,
		keys: Vec<Query>,
	) -> Result<Vec<u8>, anyhow::Error>;

	/// Query state proof for some keys, return scaled encoded proof
	async fn query_state_proof(
		&self,
		at: u64,
		keys: Vec<Vec<u8>>,
	) -> Result<Vec<u8>, anyhow::Error>;

	/// Query all ismp events on naive that can be processed for a [`StateMachineUpdated`]
	/// event on the counterparty
	async fn query_ismp_events(
		&self,
		previous_height: u64,
		event: StateMachineUpdated,
	) -> Result<Vec<Event>, anyhow::Error>;

	/// Name of this chain, used in logs.
	fn name(&self) -> String;

	/// State Machine Id for this client which would be it's state machine id
	/// on the counterparty chain
	fn state_machine_id(&self) -> StateMachineId;

	/// Should return a numerical value for the max gas allowed for transactions in a block.
	fn block_max_gas(&self) -> u64;

	/// Should return the initial height at which events should be queried
	fn initial_height(&self) -> u64;

	/// Should return a numerical estimate of the gas to be consumed for a batch of messages.
	async fn estimate_gas(
		&self,
		msg: Vec<Message>,
	) -> Result<Vec<EstimateGasReturnParams>, anyhow::Error>;

	/// Should return fee relayer would be recieving to relay a request mesage giving a hash
	/// (message commiment)
	/// Should return Erc20 standard type with 18 decimals value
	async fn query_request_fee_metadata(&self, hash: H256) -> Result<U256, anyhow::Error>;

	/// Should return fee relayer would be recieving to relay a responce mesage giving a hash
	/// (message commiment)
	/// Should return Erc20 standard type with 18 decimals value
	async fn query_response_fee_metadata(&self, hash: H256) -> Result<U256, anyhow::Error>;

	/// Return a stream that watches for updates to [`counterparty_state_id`], yields when new
	/// [`StateMachineUpdated`] event is observed for [`counterparty_state_id`]
	async fn state_machine_update_notification(
		&self,
		counterparty_state_id: StateMachineId,
	) -> Result<BoxStream<StateMachineUpdated>, anyhow::Error>;

	/// This should be used to submit new messages [`Vec<Message>`] from a counterparty chain to
	/// this chain.
	///
	/// Should only return Ok if the transaction was successfully inserted into a block.
	/// Should return a list of requests and responses that where successfully processed
	async fn submit(&self, messages: Vec<Message>) -> Result<Vec<TxReceipt>, anyhow::Error>;

	/// This method should return the key used to be used to query the state proof for the request
	/// commitment
	fn request_commitment_full_key(&self, commitment: H256) -> Vec<Vec<u8>>;

	/// This method should return the key used to be used to query the state proof for the request
	/// receipt
	fn request_receipt_full_key(&self, commitment: H256) -> Vec<Vec<u8>>;

	/// This method should return the key used to be used to query the state proof for the response
	/// commitment
	fn response_commitment_full_key(&self, commitment: H256) -> Vec<Vec<u8>>;

	/// This method should return the key used to be used to query the state proof for the response
	/// receipt
	fn response_receipt_full_key(&self, commitment: H256) -> Vec<Vec<u8>>;

	/// Relayer's address on this chain
	fn address(&self) -> Vec<u8>;

	/// Sign a prehashed message using the Relayer's private key
	fn sign(&self, msg: &[u8]) -> Signature;

	/// Set the initial height with the finalized height on counterparty
	async fn set_latest_finalized_height<P: IsmpProvider + 'static>(
		&mut self,
		counterparty: &P,
	) -> Result<(), anyhow::Error>;

	/// Set the initial consensus state for a given consensus state id on this chain
	async fn set_initial_consensus_state(
		&self,
		message: CreateConsensusState,
	) -> Result<(), anyhow::Error>;

	/// Temporary: Submit a message to freeze the State Machine
	async fn freeze_state_machine(&self, id: StateMachineId) -> Result<(), anyhow::Error>;

	/// Fetch the host manager address for this chain
	async fn query_host_manager_address(&self) -> Result<Vec<u8>, anyhow::Error>;
}

/// Provides an interface for handling byzantine behaviour. Implementations of this should watch for
/// eclipse attacks, as well as invalid state transitions.
#[async_trait::async_trait]
pub trait ByzantineHandler {
	/// Returns the [`ConsensusMessage`] that caused the emission of  [`StateMachineUpdated`]
	/// event
	async fn query_consensus_message(
		&self,
		challenge_event: StateMachineUpdated,
	) -> Result<ConsensusMessage, anyhow::Error>;

	/// Check the client message for byzantine behaviour and submit it to the chain if any.
	async fn check_for_byzantine_attack<C: IsmpHost + IsmpProvider>(
		&self,
		counterparty: &C,
		consensus_message: ConsensusMessage,
	) -> Result<(), anyhow::Error>;
}

/// Provides an interface for the chain to the relayer core for submitting Ismp messages as well as
#[async_trait::async_trait]
pub trait IsmpHost: ByzantineHandler + Clone + Send + Sync {
	/// Return a stream that yields [`ConsensusMessage`] when a new consensus update
	/// can be sent to the counterparty
	async fn consensus_notification<C>(
		&self,
		counterparty: C,
	) -> Result<BoxStream<ConsensusMessage>, anyhow::Error>
	where
		C: IsmpHost + IsmpProvider + Clone + 'static;

	/// Query the trusted consensus state for this host
	async fn query_initial_consensus_state(
		&self,
	) -> Result<Option<CreateConsensusState>, anyhow::Error>;
}

#[async_trait::async_trait]
pub trait HyperbridgeClaim {
	async fn available_amount<C: IsmpProvider>(
		&self,
		_client: &C,
		_chain: &StateMachine,
	) -> anyhow::Result<U256> {
		Ok(U256::from(0))
	}
	async fn accumulate_fees(&self, proof: WithdrawalProof) -> anyhow::Result<()>;
	async fn withdraw_funds<C: IsmpProvider>(
		&self,
		client: &C,
		chain: StateMachine,
		gas_limit: u64,
	) -> anyhow::Result<WithdrawFundsResult>;
}

pub struct WithdrawFundsResult {
	/// Post request emitted by the withdraw request
	pub post: Post,
	/// Block height at which the post request was emitted
	pub block: u64,
}

#[derive(Clone, Debug)]
pub struct NonceProvider {
	nonce: Arc<tokio::sync::Mutex<u64>>,
}

impl NonceProvider {
	pub fn new(nonce: u64) -> Self {
		Self { nonce: Arc::new(tokio::sync::Mutex::new(nonce)) }
	}

	pub async fn get_nonce(&self) -> u64 {
		let mut guard = self.nonce.lock().await;
		let nonce = *guard;
		*guard = nonce + 1;
		nonce
	}

	pub async fn read_nonce(&self) -> u64 {
		let guard = self.nonce.lock().await;
		let nonce = *guard;
		nonce
	}
}
pub async fn wait_for_challenge_period<C: IsmpProvider>(
	client: &C,
	last_consensus_update: Duration,
	challenge_period: Duration,
) -> anyhow::Result<()> {
	if challenge_period != Duration::ZERO {
		log::info!("Waiting for challenge period {challenge_period:?}");
	}

	tokio::time::sleep(challenge_period).await;
	let current_timestamp = client.query_timestamp().await?;
	let mut delay = current_timestamp.saturating_sub(last_consensus_update);

	while delay <= challenge_period {
		tokio::time::sleep(challenge_period - delay).await;
		let current_timestamp = client.query_timestamp().await?;
		delay = current_timestamp.saturating_sub(last_consensus_update);
	}
	Ok(())
}
