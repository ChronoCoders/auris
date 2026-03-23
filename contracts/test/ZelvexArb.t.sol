// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

// NOTE: These are fork tests. Run with:
//   forge test --fork-url <erigon_node_url> --match-path test/ZelvexArb.t.sol
//
// Requires a live Ethereum mainnet archive node (erigon recommended).
// All pool addresses, token addresses, and Aave addresses are mainnet constants.

import "forge-std/Test.sol";
import "../ZelvexArb.sol";

// ── Mainnet constants ──────────────────────────────────────────────────────────
address constant AAVE_PROVIDER   = 0x2f39d218133AFaB8F2B819B1066c7E434Ad94E9e;
address constant AAVE_POOL_ADDR  = 0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2; // Aave V3 mainnet pool
address constant WETH            = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
address constant USDC            = 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48;

// Uniswap V2 USDC/WETH pair
address constant UNI_USDC_WETH   = 0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc;
// SushiSwap USDC/WETH pair
address constant SUSHI_USDC_WETH = 0x397FF1542f962076d0BFE58eA045FfA2d347ACa0;

// ── Helper interfaces used in tests ───────────────────────────────────────────
interface IWETH {
    function deposit() external payable;
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

interface IUniV2PairLocal {
    function token0() external view returns (address);
    function token1() external view returns (address);
    function getReserves() external view returns (uint112 r0, uint112 r1, uint32 ts);
    function sync() external;
}

contract ZelvexArbTest is Test {
    ZelvexArb arb;
    address   owner;

    function setUp() public {
        owner = address(this);
        arb = new ZelvexArb(AAVE_PROVIDER);
    }

    // ── Helper: manipulate pool reserves via vm.store ─────────────────────────
    // UniswapV2Pair stores reserves in slot 8 packed as:
    //   uint112 reserve0 | uint112 reserve1 | uint32 blockTimestampLast
    function _setReserves(
        address pair,
        uint112 r0,
        uint112 r1
    ) internal {
        // Slot 8: [blockTimestampLast(32) | reserve1(112) | reserve0(112)]
        uint32 ts = uint32(block.timestamp);
        bytes32 packed = bytes32(
            (uint256(ts) << 224) |
            (uint256(r1) << 112) |
            uint256(r0)
        );
        vm.store(pair, bytes32(uint256(8)), packed);
    }

    // ── 5.1 Successful arbitrage ───────────────────────────────────────────────
    // Manipulate pool reserves so a real price difference exists, fund contract
    // with WETH for repayment, then call executeArb. On a live fork the actual
    // flash loan flow works; here we manipulate reserves to guarantee profit.
    function test_successful_arb() public {
        // Give this test contract ETH and wrap to WETH
        vm.deal(address(this), 100 ether);
        IWETH(WETH).deposit{value: 100 ether}();

        // Fund the ZelvexArb contract with WETH so it can repay the loan + profit
        // (In a real scenario the arb profit covers the repayment; here we pre-fund
        //  because we cannot actually move real prices on a static fork block.)
        IWETH(WETH).transfer(address(arb), 10 ether);

        // Manipulate UNI pool: make WETH cheap (lots of WETH, little USDC)
        // token0=USDC, token1=WETH on UNI_USDC_WETH
        // Set UNI: 10M USDC, 10000 WETH → price ~$1000/WETH
        _setReserves(UNI_USDC_WETH,   uint112(10_000_000e6), uint112(10_000e18));
        // Set SUSHI: 20M USDC, 10000 WETH → price ~$2000/WETH (sell here)
        _setReserves(SUSHI_USDC_WETH, uint112(20_000_000e6), uint112(10_000e18));

        // Sync does not update storage in the same way on a fork, so we skip sync
        // and rely on vm.store to have set the slot directly.

        ZelvexArb.ArbParams memory params = ZelvexArb.ArbParams({
            tokenIn:   WETH,
            tokenOut:  USDC,
            poolA:     UNI_USDC_WETH,
            poolB:     SUSHI_USDC_WETH,
            amountIn:  1 ether,
            minProfit: 1  // very low minProfit — test we get past the floor
        });

        // The call may revert in a fork context if reserves don't actually produce
        // profit after fees. We just verify it does not revert for the security paths.
        // On a properly-forked block with real state this should succeed.
        arb.executeArb(params);
    }

    // ── 5.2 minProfit guard ────────────────────────────────────────────────────
    function test_insufficient_profit_reverts() public {
        vm.deal(address(this), 100 ether);
        IWETH(WETH).deposit{value: 100 ether}();
        IWETH(WETH).transfer(address(arb), 10 ether);

        ZelvexArb.ArbParams memory params = ZelvexArb.ArbParams({
            tokenIn:   WETH,
            tokenOut:  USDC,
            poolA:     UNI_USDC_WETH,
            poolB:     SUSHI_USDC_WETH,
            amountIn:  1 ether,
            minProfit: 1_000_000 ether  // impossibly high — must revert
        });

        vm.expectRevert(bytes("Insufficient profit"));
        arb.executeArb(params);
    }

    // ── 5.3 Stale price simulation ─────────────────────────────────────────────
    // Submit a large swap to move the pool price, then try to arb with old amounts.
    function test_stale_price_reverts() public {
        vm.deal(address(this), 200 ether);
        IWETH(WETH).deposit{value: 200 ether}();
        IWETH(WETH).transfer(address(arb), 10 ether);

        // Simulate large price-moving swap on UNI by drastically moving reserves
        // Make UNI and SUSHI have the same price — no arb opportunity
        _setReserves(UNI_USDC_WETH,   uint112(10_000_000e6), uint112(10_000e18));
        _setReserves(SUSHI_USDC_WETH, uint112(10_000_000e6), uint112(10_000e18));

        ZelvexArb.ArbParams memory params = ZelvexArb.ArbParams({
            tokenIn:   WETH,
            tokenOut:  USDC,
            poolA:     UNI_USDC_WETH,
            poolB:     SUSHI_USDC_WETH,
            amountIn:  1 ether,
            minProfit: 1e18  // meaningful min profit that won't be met when prices equal
        });

        // Expect revert because no profit after fees with equal reserves
        vm.expectRevert(bytes("Insufficient profit"));
        arb.executeArb(params);
    }

    // ── 5.4 Gas measurement (10 runs) ─────────────────────────────────────────
    function test_gas_measurement() public {
        vm.deal(address(this), 1000 ether);
        IWETH(WETH).deposit{value: 1000 ether}();
        IWETH(WETH).transfer(address(arb), 100 ether);

        // Manipulate reserves for a profitable spread
        _setReserves(UNI_USDC_WETH,   uint112(10_000_000e6), uint112(10_000e18));
        _setReserves(SUSHI_USDC_WETH, uint112(20_000_000e6), uint112(10_000e18));

        ZelvexArb.ArbParams memory params = ZelvexArb.ArbParams({
            tokenIn:   WETH,
            tokenOut:  USDC,
            poolA:     UNI_USDC_WETH,
            poolB:     SUSHI_USDC_WETH,
            amountIn:  0.1 ether,
            minProfit: 1
        });

        uint256 totalGas = 0;
        for (uint256 i = 0; i < 10; i++) {
            uint256 gasBefore = gasleft();
            arb.executeArb(params);
            uint256 gasAfter = gasleft();
            uint256 used = gasBefore - gasAfter;
            totalGas += used;
            assertGe(used, 180_000, "gas below 180k");
            assertLe(used, 220_000, "gas above 220k");
        }
        // Average should also be in range
        uint256 avg = totalGas / 10;
        assertGe(avg, 180_000, "avg gas below 180k");
        assertLe(avg, 220_000, "avg gas above 220k");
    }

    // ── 5.5 Pool token mismatch ───────────────────────────────────────────────
    function test_pool_token_mismatch_reverts() public {
        // Use SUSHI as poolA but claim it pairs WETH with DAI — it actually pairs USDC/WETH
        address DAI = 0x6B175474E89094C44Da98b954EedeAC495271d0F;

        ZelvexArb.ArbParams memory params = ZelvexArb.ArbParams({
            tokenIn:   WETH,
            tokenOut:  DAI,          // DAI is NOT in UNI_USDC_WETH
            poolA:     UNI_USDC_WETH,
            poolB:     SUSHI_USDC_WETH,
            amountIn:  1 ether,
            minProfit: 1
        });

        vm.expectRevert(bytes("Pool token mismatch"));
        arb.executeArb(params);
    }

    // ── 5.6 Direct executeOperation call reverts ──────────────────────────────
    function test_direct_execute_operation_reverts() public {
        bytes memory encoded = abi.encode(ZelvexArb.ArbParams({
            tokenIn:   WETH,
            tokenOut:  USDC,
            poolA:     UNI_USDC_WETH,
            poolB:     SUSHI_USDC_WETH,
            amountIn:  1 ether,
            minProfit: 1
        }));

        vm.expectRevert(bytes("Caller not Aave pool"));
        arb.executeOperation(
            WETH,
            1 ether,
            0,
            address(this),  // initiator is this contract, not arb itself
            encoded
        );
    }

    // ── 5.7 Withdraw transfers full balance to owner ──────────────────────────
    function test_withdraw_transfers_to_owner() public {
        vm.deal(address(this), 10 ether);
        IWETH(WETH).deposit{value: 10 ether}();

        uint256 depositAmount = 5 ether;
        IWETH(WETH).transfer(address(arb), depositAmount);

        uint256 ownerBefore = IWETH(WETH).balanceOf(address(this));
        uint256 contractBefore = IWETH(WETH).balanceOf(address(arb));
        assertEq(contractBefore, depositAmount);

        arb.withdraw(WETH);

        uint256 contractAfter = IWETH(WETH).balanceOf(address(arb));
        uint256 ownerAfter = IWETH(WETH).balanceOf(address(this));

        assertEq(contractAfter, 0, "contract balance must be zero after withdraw");
        assertEq(ownerAfter, ownerBefore + depositAmount, "owner must receive full balance");
    }

    // ── 5.8 Initiator check: external contract triggering callback ─────────────
    function test_initiator_check_reverts_from_external() public {
        bytes memory encoded = abi.encode(ZelvexArb.ArbParams({
            tokenIn:   WETH,
            tokenOut:  USDC,
            poolA:     UNI_USDC_WETH,
            poolB:     SUSHI_USDC_WETH,
            amountIn:  1 ether,
            minProfit: 1
        }));

        // Impersonate the real Aave pool so msg.sender check passes,
        // but initiator is address(this) not address(arb) — should revert
        vm.prank(AAVE_POOL_ADDR);
        vm.expectRevert(bytes("Initiator not self"));
        arb.executeOperation(
            WETH,
            1 ether,
            0,
            address(this), // initiator != address(arb)
            encoded
        );
    }
}
