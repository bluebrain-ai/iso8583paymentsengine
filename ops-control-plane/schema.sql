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
