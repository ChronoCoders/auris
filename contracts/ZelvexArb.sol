// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IPoolAddressesProvider {
    function getPool() external view returns (address);
}

interface IPool {
    function flashLoanSimple(
        address receiverAddress,
        address asset,
        uint256 amount,
        bytes calldata params,
        uint16 referralCode
    ) external;
}

interface IUniswapV2Pair {
    function token0() external view returns (address);
    function token1() external view returns (address);
    function swap(
        uint amount0Out,
        uint amount1Out,
        address to,
        bytes calldata data
    ) external;
    function getReserves() external view returns (
        uint112 reserve0,
        uint112 reserve1,
        uint32 blockTimestampLast
    );
}

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

contract ZelvexArb {
    address public immutable owner;
    IPool   public immutable aavePool;

    uint256 private constant AAVE_FEE_BPS = 5;   // 0.05%
    uint256 private constant UNI_FEE_NUM  = 997;
    uint256 private constant UNI_FEE_DEN  = 1000;

    struct ArbParams {
        address tokenIn;
        address tokenOut;
        address poolA;      // buy pool  (tokenIn  → tokenOut)
        address poolB;      // sell pool (tokenOut → tokenIn)
        uint256 amountIn;
        uint256 minProfit;  // revert if net profit < this (in tokenIn units)
    }

    modifier onlyOwner() {
        require(msg.sender == owner, "Not owner");
        _;
    }

    constructor(address _aaveProvider) {
        owner    = msg.sender;
        aavePool = IPool(IPoolAddressesProvider(_aaveProvider).getPool());
    }

    // ─── Entry point called by bot ───────────────────────────────────────────

    function executeArb(ArbParams calldata params) external onlyOwner {
        require(params.minProfit > 0,        "minProfit must be > 0");
        require(params.poolA != params.poolB, "Pools must differ");
        require(params.tokenIn != params.tokenOut, "Tokens must differ");

        bytes memory encoded = abi.encode(params);
        aavePool.flashLoanSimple(
            address(this),
            params.tokenIn,
            params.amountIn,
            encoded,
            0
        );
    }

    // ─── Aave callback ───────────────────────────────────────────────────────

    function executeOperation(
        address asset,
        uint256 amount,
        uint256 premium,
        address initiator,
        bytes calldata params
    ) external returns (bool) {
        // Validate caller and initiator
        require(msg.sender  == address(aavePool), "Caller not Aave pool");
        require(initiator   == address(this),     "Initiator not self");

        ArbParams memory p = abi.decode(params, (ArbParams));

        // Validate loan matches what we requested
        require(asset  == p.tokenIn,  "Asset mismatch");
        require(amount == p.amountIn, "Amount mismatch");

        // Validate pool token composition before swapping
        _validatePool(p.poolA, p.tokenIn,  p.tokenOut);
        _validatePool(p.poolB, p.tokenOut, p.tokenIn);

        // Swap on Pool A: tokenIn → tokenOut
        uint256 received = _swap(p.poolA, p.tokenIn, p.tokenOut, amount);

        // Swap on Pool B: tokenOut → tokenIn
        uint256 returned = _swap(p.poolB, p.tokenOut, p.tokenIn, received);

        // Validate profit meets floor
        uint256 repayAmount = amount + premium;
        require(
            returned >= repayAmount + p.minProfit,
            "Insufficient profit"
        );

        // Approve repayment
        IERC20(asset).approve(address(aavePool), repayAmount);

        return true;
    }

    // ─── Internal helpers ────────────────────────────────────────────────────

    /// @dev Validates that tokenA and tokenB are the two tokens in the pair.
    function _validatePool(
        address pair,
        address tokenA,
        address tokenB
    ) internal view {
        address t0 = IUniswapV2Pair(pair).token0();
        address t1 = IUniswapV2Pair(pair).token1();
        require(
            (tokenA == t0 && tokenB == t1) || (tokenA == t1 && tokenB == t0),
            "Pool token mismatch"
        );
    }

    function _swap(
        address pair,
        address tokenIn,
        address tokenOut,
        uint256 amountIn
    ) internal returns (uint256 amountOut) {
        (uint112 r0, uint112 r1,) = IUniswapV2Pair(pair).getReserves();
        address t0 = IUniswapV2Pair(pair).token0();

        (uint112 rIn, uint112 rOut) = (tokenIn == t0)
            ? (r0, r1)
            : (r1, r0);

        amountOut = _getAmountOut(amountIn, uint256(rIn), uint256(rOut));
        require(amountOut > 0, "Zero output");

        IERC20(tokenIn).transfer(pair, amountIn);

        (uint256 out0, uint256 out1) = (tokenIn == t0)
            ? (uint256(0), amountOut)
            : (amountOut, uint256(0));

        IUniswapV2Pair(pair).swap(out0, out1, address(this), "");
    }

    function _getAmountOut(
        uint256 amountIn,
        uint256 reserveIn,
        uint256 reserveOut
    ) internal pure returns (uint256) {
        require(reserveIn > 0 && reserveOut > 0, "Zero reserve");
        uint256 amountInWithFee = amountIn * UNI_FEE_NUM;
        uint256 numerator       = amountInWithFee * reserveOut;
        uint256 denominator     = reserveIn * UNI_FEE_DEN + amountInWithFee;
        return numerator / denominator;
    }

    // ─── Owner utilities ─────────────────────────────────────────────────────

    /// Withdraw ERC-20 profit to owner
    function withdraw(address token) external onlyOwner {
        uint256 bal = IERC20(token).balanceOf(address(this));
        require(bal > 0, "Nothing to withdraw");
        require(IERC20(token).transfer(owner, bal), "Transfer failed");
    }

    /// Rescue ETH accidentally sent to contract
    function rescueEth() external onlyOwner {
        payable(owner).transfer(address(this).balance);
    }
}
