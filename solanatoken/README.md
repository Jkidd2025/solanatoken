# solanatoken

Name: Next Gen Crypto (NGC)
Symbol: NGC
Total Supply: 1,000,000,000 (1 Billion)
Decimals: 6
Smallest Unit: 0.000001 NGC

// SPL Token Configuration
const tokenConfig = {
name: "Next Gen Crypto",
symbol: "NGC",
decimals: 6,
totalSupply: 1_000_000_000 \* 10\*\*6, // Adjusting for 6 decimals
mintAuthority: null, // No minting after initial supply
freezeAuthority: null // No freeze authority
};
