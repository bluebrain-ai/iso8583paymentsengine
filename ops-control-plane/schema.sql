CREATE TABLE IF NOT EXISTS crdb_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pan_hash VARCHAR(255) NOT NULL,
    daily_limit BIGINT NOT NULL,
    status VARCHAR(50) NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    valid_from DATETIME DEFAULT CURRENT_TIMESTAMP,
    valid_to DATETIME,
    is_live BOOLEAN NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS stip_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    network_id VARCHAR(50) NOT NULL,
    condition_type VARCHAR(100) NOT NULL,
    max_approval_amount BIGINT NOT NULL,
    max_risk_score INTEGER NOT NULL DEFAULT 100,
    version INTEGER NOT NULL DEFAULT 1,
    valid_from DATETIME DEFAULT CURRENT_TIMESTAMP,
    valid_to DATETIME,
    is_live BOOLEAN NOT NULL DEFAULT 1
);

-- Indexes for performance
CREATE INDEX IF NOT EXISTS idx_crdb_live ON crdb_records(is_live);
CREATE INDEX IF NOT EXISTS idx_stip_live ON stip_rules(is_live);

-- Seed Minimal Test Data
INSERT INTO crdb_records (pan_hash, daily_limit, status, version, is_live)
SELECT '4111222233334444', 150000, 'Active', 1, 1 
WHERE NOT EXISTS (SELECT 1 FROM crdb_records);

INSERT INTO crdb_records (pan_hash, daily_limit, status, version, is_live)
SELECT '5555666677778888', 5000, 'Blocked', 1, 1 
WHERE NOT EXISTS (SELECT 1 FROM crdb_records WHERE pan_hash = '5555666677778888');

INSERT INTO stip_rules (network_id, condition_type, max_approval_amount, max_risk_score, version, is_live)
SELECT 'VISA', 'STAND_IN_APPROVAL', 5000, 85, 1, 1
WHERE NOT EXISTS (SELECT 1 FROM stip_rules);

INSERT INTO stip_rules (network_id, condition_type, max_approval_amount, max_risk_score, version, is_live)
SELECT 'MASTERCARD', 'STAND_IN_APPROVAL', 2500, 85, 1, 1
WHERE NOT EXISTS (SELECT 1 FROM stip_rules WHERE network_id = 'MASTERCARD');

CREATE TABLE IF NOT EXISTS routing_rules (
    id SERIAL PRIMARY KEY,
    bin_prefix VARCHAR(19) NOT NULL,
    destination_type VARCHAR(50) NOT NULL,
    target_node VARCHAR(100),
    failover_node VARCHAR(100),
    version INT NOT NULL DEFAULT 1,
    valid_from TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    valid_to TIMESTAMP,
    is_live BOOLEAN NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_routing_live ON routing_rules(is_live);

-- Default BIN prefix routes: broad coverage for Visa (4*) and Mastercard (5*)
INSERT INTO routing_rules (bin_prefix, destination_type, target_node, failover_node, version, is_live)
SELECT '4', 'ExternalNode', 'MockVisaNode', NULL, 1, 1
WHERE NOT EXISTS (SELECT 1 FROM routing_rules WHERE bin_prefix = '4');

INSERT INTO routing_rules (bin_prefix, destination_type, target_node, failover_node, version, is_live)
SELECT '5', 'ExternalNode', 'MockMastercardNode', NULL, 1, 1
WHERE NOT EXISTS (SELECT 1 FROM routing_rules WHERE bin_prefix = '5');

-- Seed an 8-digit BIN example to validate longest-prefix-match depth.
-- The BinTrie already supports arbitrary-length prefixes; this seed confirms
-- an 8-digit prefix overrides a shorter 1-digit prefix for matching cards.
INSERT INTO routing_rules (bin_prefix, destination_type, target_node, failover_node, version, is_live)
SELECT '41111234', 'ExternalNode', 'MockVisaNode', NULL, 1, 1
WHERE NOT EXISTS (SELECT 1 FROM routing_rules WHERE bin_prefix = '41111234');

-- PILLAR 5: Ledger, Compliance, and Settlement Precision

-- 1. Compliance Interceptor
CREATE TABLE IF NOT EXISTS sanctions_list (
    id SERIAL PRIMARY KEY,
    list_type VARCHAR(50) NOT NULL, -- e.g., 'currency', 'country', 'bin'
    list_value VARCHAR(100) NOT NULL,
    action VARCHAR(50) NOT NULL DEFAULT 'block',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    is_live BOOLEAN NOT NULL DEFAULT 1
);

-- Seed basic sanctions
INSERT INTO sanctions_list (list_type, list_value, action)
SELECT 'currency', 'RUB', 'block' WHERE NOT EXISTS (SELECT 1 FROM sanctions_list WHERE list_value = 'RUB');
INSERT INTO sanctions_list (list_type, list_value, action)
SELECT 'country', 'IR', 'block' WHERE NOT EXISTS (SELECT 1 FROM sanctions_list WHERE list_value = 'IR');

-- 2. Dynamic Fee Engine
CREATE TABLE IF NOT EXISTS fee_matrix (
    id SERIAL PRIMARY KEY,
    mcc VARCHAR(4),
    bin_prefix VARCHAR(10),
    base_pct DECIMAL(5,4) NOT NULL DEFAULT 0.00,
    flat_fee_cents INTEGER NOT NULL DEFAULT 0,
    fx_markup_pct DECIMAL(5,4) NOT NULL DEFAULT 0.00,
    is_live BOOLEAN NOT NULL DEFAULT 1
);

-- Default fee (fallback network fee)
INSERT INTO fee_matrix (mcc, bin_prefix, base_pct, flat_fee_cents, fx_markup_pct)
SELECT NULL, NULL, 0.015, 10, 0.02 WHERE NOT EXISTS (SELECT 1 FROM fee_matrix);

-- 3. Chart of Accounts & General Ledger
CREATE TABLE IF NOT EXISTS chart_of_accounts (
    id SERIAL PRIMARY KEY,
    institution_id VARCHAR(50) NOT NULL, -- e.g., 'CITI', 'DANSKE'
    currency VARCHAR(3) NOT NULL,
    vostro_account_id VARCHAR(100) NOT NULL,
    nostro_account_id VARCHAR(100) NOT NULL,
    is_live BOOLEAN NOT NULL DEFAULT 1
);

-- Seed institutions for dashboard
INSERT INTO chart_of_accounts (institution_id, currency, vostro_account_id, nostro_account_id)
SELECT 'DANSKE_BANK', 'USD', 'VOSTRO_DANSKE_USD_01', 'NOSTRO_DANSKE_USD_01' WHERE NOT EXISTS (SELECT 1 FROM chart_of_accounts WHERE institution_id = 'DANSKE_BANK');
INSERT INTO chart_of_accounts (institution_id, currency, vostro_account_id, nostro_account_id)
SELECT 'CITI', 'EUR', 'VOSTRO_CITI_EUR_01', 'NOSTRO_CITI_EUR_01' WHERE NOT EXISTS (SELECT 1 FROM chart_of_accounts WHERE institution_id = 'CITI');

CREATE TABLE IF NOT EXISTS general_ledger (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_stan VARCHAR(50) NOT NULL,
    trace_rrn VARCHAR(50) NOT NULL,
    masked_pan VARCHAR(50) NOT NULL,
    routing_bin VARCHAR(20) NOT NULL,
    source_account_id VARCHAR(100),
    target_account_id VARCHAR(100),
    principal_amount BIGINT NOT NULL,
    fee_amount BIGINT NOT NULL,
    fx_rate DECIMAL(10,5) NOT NULL DEFAULT 1.00000,
    status VARCHAR(20) NOT NULL DEFAULT 'shadow', -- 'shadow' or 'settled'
    booking_timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    institution_id VARCHAR(50)
);

CREATE INDEX IF NOT EXISTS idx_gl_status ON general_ledger(status);
CREATE INDEX IF NOT EXISTS idx_gl_institution ON general_ledger(institution_id);
