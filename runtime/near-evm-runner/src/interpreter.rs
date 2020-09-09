use std::sync::Arc;

use ethereum_types::{Address, U256};
use evm::{CreateContractAddress, Factory};
use vm::{ActionParams, ActionValue, CallType, Ext, GasLeft, ParamsType, ReturnData, Schedule};

use near_vm_errors::{EvmError, VMLogicError};

use crate::evm_state::{EvmState, StateStore, SubState};
use crate::near_ext::NearExt;
use crate::types::Result;
use crate::utils;

const PREPAID_GAS: u128 = 1_000_000_000_000;

pub fn deploy_code<T: EvmState>(
    state: &mut T,
    origin: &Address,
    sender: &Address,
    value: U256,
    call_stack_depth: usize,
    address_type: CreateContractAddress,
    recreate: bool,
    code: &[u8],
) -> Result<Address> {
    let mut nonce = U256::default();
    if address_type == CreateContractAddress::FromSenderAndNonce {
        nonce = state.next_nonce(&sender)?;
    };
    let (address, _) = utils::evm_contract_address(address_type, &sender, &nonce, &code);

    if recreate {
        state.recreate(address.0);
    } else if state.code_at(&address)?.is_some() {
        return Err(VMLogicError::EvmError(EvmError::DuplicateContract(hex::encode(address.0))));
    }

    let (result, state_updates) =
        _create(state, origin, sender, value, call_stack_depth, &address, code)?;

    // Apply known gas amount changes (all reverts are NeedsReturn)
    // Apply NeedsReturn changes if apply_state
    // Return the result unmodified
    let gas_used: U256;
    let (return_data, apply) = match result {
        Some(GasLeft::Known(gas_left)) => {
            gas_used = U256::from(PREPAID_GAS) - gas_left;
            (ReturnData::empty(), true)
        },
        Some(GasLeft::NeedsReturn { gas_left, data, apply_state }) => {
            gas_used = U256::from(PREPAID_GAS) - gas_left;
            (data, apply_state)
        },
        _ => return Err(VMLogicError::EvmError(EvmError::UnknownError)),
    };

    println!("Gas used in deploy_code: {}", gas_used);
    if apply {
        state.commit_changes(&state_updates.unwrap())?;
        state.set_code(&address, &return_data.to_vec())?;
    } else {
        return Err(VMLogicError::EvmError(EvmError::DeployFail(hex::encode(
            return_data.to_vec(),
        ))));
    }
    Ok(address)
}

pub fn _create<T: EvmState>(
    state: &mut T,
    origin: &Address,
    sender: &Address,
    value: U256,
    call_stack_depth: usize,
    address: &Address,
    code: &[u8],
) -> Result<(Option<GasLeft>, Option<StateStore>)> {
    let mut store = StateStore::default();
    let mut sub_state = SubState::new(sender, &mut store, state);

    let params = ActionParams {
        code_address: *address,
        address: *address,
        sender: *sender,
        origin: *origin,
        // TODO: gas usage.
        gas: PREPAID_GAS.into(),
        gas_price: 1.into(),
        value: ActionValue::Transfer(value),
        code: Some(Arc::new(code.to_vec())),
        code_hash: None,
        data: None,
        call_type: CallType::None,
        params_type: vm::ParamsType::Embedded,
    };

    sub_state.transfer_balance(sender, address, value)?;

    let mut ext = NearExt::new(*address, *origin, &mut sub_state, call_stack_depth + 1, false);
    // TODO: gas usage.
    ext.info.gas_limit = U256::from(PREPAID_GAS);
    ext.schedule = Schedule::new_constantinople();

    let instance = Factory::default().create(params, ext.schedule(), ext.depth());

    // Run the code
    let result = instance.exec(&mut ext);

    Ok((result.ok().unwrap().ok(), Some(store)))
}

#[allow(clippy::too_many_arguments)]
pub fn call<T: EvmState>(
    state: &mut T,
    origin: &Address,
    sender: &Address,
    value: Option<U256>,
    call_stack_depth: usize,
    contract_address: &Address,
    input: &[u8],
    should_commit: bool,
) -> Result<ReturnData> {
    run_and_commit_if_success(
        state,
        origin,
        sender,
        value,
        call_stack_depth,
        CallType::Call,
        contract_address,
        contract_address,
        input,
        false,
        should_commit,
    )
}

pub fn delegate_call<T: EvmState>(
    state: &mut T,
    origin: &Address,
    sender: &Address,
    call_stack_depth: usize,
    context: &Address,
    delegee: &Address,
    input: &[u8],
) -> Result<ReturnData> {
    run_and_commit_if_success(
        state,
        origin,
        sender,
        None,
        call_stack_depth,
        CallType::DelegateCall,
        context,
        delegee,
        input,
        false,
        true,
    )
}

pub fn static_call<T: EvmState>(
    state: &mut T,
    origin: &Address,
    sender: &Address,
    call_stack_depth: usize,
    contract_address: &Address,
    input: &[u8],
) -> Result<ReturnData> {
    run_and_commit_if_success(
        state,
        origin,
        sender,
        None,
        call_stack_depth,
        CallType::StaticCall,
        contract_address,
        contract_address,
        input,
        true,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_and_commit_if_success<T: EvmState>(
    state: &mut T,
    origin: &Address,
    sender: &Address,
    value: Option<U256>,
    call_stack_depth: usize,
    call_type: CallType,
    state_address: &Address,
    code_address: &Address,
    input: &[u8],
    is_static: bool,
    should_commit: bool,
) -> Result<ReturnData> {
    // run the interpreter and
    let (result, state_updates) = run_against_state(
        state,
        origin,
        sender,
        value,
        call_stack_depth,
        call_type,
        state_address,
        code_address,
        input,
        is_static,
    )?;

    // Apply known gas amount changes (all reverts are NeedsReturn)
    // Apply NeedsReturn changes if apply_state
    // Return the result unmodified
    let gas_used: U256;
    let return_data = match result {
        Some(GasLeft::Known(gas_left)) => {
            println!("-=-=-= {} {}", PREPAID_GAS, gas_left);
            gas_used = U256::from(PREPAID_GAS) - gas_left;
            Ok(ReturnData::empty())
        },
        Some(GasLeft::NeedsReturn { gas_left, data, apply_state: true }) => {
            println!("-=-=-= {} {}", PREPAID_GAS, gas_left);

            gas_used = U256::from(PREPAID_GAS) - gas_left;
            Ok(data)
        },
        Some(GasLeft::NeedsReturn { gas_left, data, apply_state: false }) => {
            println!("-=-=-= {} {}", PREPAID_GAS, gas_left);

            gas_used = U256::from(PREPAID_GAS) - gas_left;
            Err(VMLogicError::EvmError(EvmError::Revert(hex::encode(data.to_vec()))))
        },
        _ => return Err(VMLogicError::EvmError(EvmError::UnknownError)),
    };
    println!("Gas used in run_and_commit_if_success: {}", gas_used);

    // Don't apply changes from a static context (these _should_ error in the ext)
    if !is_static && return_data.is_ok() && should_commit {
        state.commit_changes(&state_updates.unwrap())?;
    }

    return_data
}

/// Runs the interpreter. Produces state diffs
#[allow(clippy::too_many_arguments)]
fn run_against_state<T: EvmState>(
    state: &mut T,
    origin: &Address,
    sender: &Address,
    value: Option<U256>,
    call_stack_depth: usize,
    call_type: CallType,
    state_address: &Address,
    code_address: &Address,
    input: &[u8],
    is_static: bool,
) -> Result<(Option<GasLeft>, Option<StateStore>)> {
    let code = state.code_at(code_address)?.unwrap_or_else(Vec::new);

    let mut store = StateStore::default();
    let mut sub_state = SubState::new(sender, &mut store, state);

    let mut params = ActionParams {
        code_address: *code_address,
        code_hash: None,
        address: *state_address,
        sender: *sender,
        origin: *origin,
        // TODO: gas usage.
        gas: PREPAID_GAS.into(),
        gas_price: 1.into(),
        value: ActionValue::Apparent(0.into()),
        code: Some(Arc::new(code)),
        data: Some(input.to_vec()),
        call_type,
        params_type: ParamsType::Separate,
    };

    if let Some(val) = value {
        params.value = ActionValue::Transfer(val);
        // substate transfer will get reverted if the call fails
        sub_state.transfer_balance(sender, state_address, val)?;
    }

    let mut ext =
        NearExt::new(*state_address, *origin, &mut sub_state, call_stack_depth + 1, is_static);
    // TODO: gas usage.
    ext.info.gas_limit = U256::from(PREPAID_GAS);
    ext.schedule = Schedule::new_constantinople();

    let instance = Factory::default().create(params, ext.schedule(), ext.depth());

    // Run the code
    let result = instance.exec(&mut ext);
    Ok((result.ok().unwrap().ok(), Some(store)))
}
