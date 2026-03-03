# Secure Signer Local Setup

1. Start anvil: `anvil --fork-url https://rpc.hoodi.ethpandaops.io`
2. Use `atakit registry ls` to get list of mock smart contracts. Use the `SessionRegistryMock` address when deploying puffer-contracts.
3. Deploy Puffer contracts. Clone puffer-contracts repo and run `yarn install` in root and run `forge install` in mainnet-contracts/.

   ```sh
   export DEV_WALLET=your-wallet-from-anvil
   export SESSION_REGISTRY=0xD1860020870ffEd23a644d0CD4CA9E7b3Ff53D6c
   export FRESHNESS_BLOCKS=10
   export DEV_WALLET_PK=your-pk-from-anvil
   export PK=$DEV_WALLET_PK

   forge script script/DeployEverything.s.sol:DeployEverything \
     --private-key ${DEV_WALLET_PK} \
     --rpc-url http://127.0.0.1:8545 \
     --sig 'run(address, address[] calldata, uint256, address, uint256)' $SESSION_REGISTRY "[$DEV_WALLET]" 1 $DEV_WALLET $FRESHNESS_BLOCKS
   ```

   Once deployment is done, scroll to the top to see all the deployed addresses. `GuardianModule` address is needed to call `GuardianModule.setAllowedWorkload`.

4. Start sim-agent: `atakit sim-agent --rpc-url http://localhost:8545`
5. Copy the workload id from sim-agent logs.

   ```text
   Workload: secure-signer:dev-20260302 (workload_id: 0xf038944cba05970c3ae785a127795a4e29271417663caa17464cb31d42a5b2f9)
   Workload: validator:dev-20260302 (workload_id: 0xd230a02fc0e67da1d6f4633fa2ff403a04669ec37b0fe9f522b043ec5ab6fcbf)
   Workload: guardian:dev-20260302 (workload_id: 0x2b16a66adfbf002b4367d9d831d02129976af0c5396dc742cd0d6053198d3bb2)
   ```

6. Call `GuardianModule.setAllowedWorkload`.

   ```sh
   cast send 0x457cCf29090fe5A24c19c1bc95F492168C0EaFdb \
     "setAllowedWorkload(bytes32,bool)" \
     <YOUR_WORKLOAD_ID> \
     true \
     --rpc-url http://localhost:8545 \
     --private-key $DEV_WALLET_PK
   ```

7. Now confirm if workload is allowed.

   ```sh
   cast call 0x457cCf29090fe5A24c19c1bc95F492168C0EaFdb "isWorkloadAllowed(bytes32)" 0x2b16a66adfbf002b4367d9d831d02129976af0c5396dc742cd0d6053198d3bb2 --rpc-url http://localhost:8545
   ```
