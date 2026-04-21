-- OFAC / Sanctions Rules (Pillar 5)
INSERT INTO sanctions_list (list_type, list_value, action) 
SELECT 'Country', 'NK', 'Block' WHERE NOT EXISTS (SELECT 1 FROM sanctions_list WHERE list_value = 'NK');

INSERT INTO sanctions_list (list_type, list_value, action) 
SELECT 'Country', 'SY', 'Block' WHERE NOT EXISTS (SELECT 1 FROM sanctions_list WHERE list_value = 'SY');

INSERT INTO sanctions_list (list_type, list_value, action) 
SELECT 'Currency', 'CUP', 'Block' WHERE NOT EXISTS (SELECT 1 FROM sanctions_list WHERE list_value = 'CUP');

INSERT INTO sanctions_list (list_type, list_value, action) 
SELECT 'BIN', '420000', 'Alert' WHERE NOT EXISTS (SELECT 1 FROM sanctions_list WHERE list_value = '420000');

INSERT INTO sanctions_list (list_type, list_value, action) 
SELECT 'Merchant', '5933', 'Block' WHERE NOT EXISTS (SELECT 1 FROM sanctions_list WHERE list_value = '5933');


-- Fee Engine Rules (Pillar 5)
INSERT INTO fee_matrix (mcc, bin_prefix, base_pct, flat_fee_cents, fx_markup_pct) 
SELECT '7995', NULL, 0.035, 50, 0.04 WHERE NOT EXISTS (SELECT 1 FROM fee_matrix WHERE mcc = '7995');

INSERT INTO fee_matrix (mcc, bin_prefix, base_pct, flat_fee_cents, fx_markup_pct) 
SELECT '5812', NULL, 0.012, 10, 0.01 WHERE NOT EXISTS (SELECT 1 FROM fee_matrix WHERE mcc = '5812');

INSERT INTO fee_matrix (mcc, bin_prefix, base_pct, flat_fee_cents, fx_markup_pct) 
SELECT NULL, '422222', 0.005, 0, 0.00 WHERE NOT EXISTS (SELECT 1 FROM fee_matrix WHERE bin_prefix = '422222');

INSERT INTO fee_matrix (mcc, bin_prefix, base_pct, flat_fee_cents, fx_markup_pct) 
SELECT '4511', NULL, 0.025, 25, 0.03 WHERE NOT EXISTS (SELECT 1 FROM fee_matrix WHERE mcc = '4511');


-- Chart of Accounts Routing (CoA)
INSERT INTO chart_of_accounts (institution_id, currency, vostro_account_id, nostro_account_id) 
SELECT 'CHASE', 'USD', 'VOSTRO_CHASE_USD_MAIN', 'NOSTRO_CHASE_USD_MAIN' WHERE NOT EXISTS (SELECT 1 FROM chart_of_accounts WHERE institution_id = 'CHASE');

INSERT INTO chart_of_accounts (institution_id, currency, vostro_account_id, nostro_account_id) 
SELECT 'BARCLAYS', 'GBP', 'VOSTRO_BARCLAYS_GBP_01', 'NOSTRO_BARCLAYS_GBP_01' WHERE NOT EXISTS (SELECT 1 FROM chart_of_accounts WHERE institution_id = 'BARCLAYS');

INSERT INTO chart_of_accounts (institution_id, currency, vostro_account_id, nostro_account_id) 
SELECT 'BOFA', 'USD', 'VOSTRO_BOFA_USD_77', 'NOSTRO_BOFA_USD_77' WHERE NOT EXISTS (SELECT 1 FROM chart_of_accounts WHERE institution_id = 'BOFA');

INSERT INTO chart_of_accounts (institution_id, currency, vostro_account_id, nostro_account_id) 
SELECT 'HSBC', 'HKD', 'VOSTRO_HSBC_HKD_08', 'NOSTRO_HSBC_HKD_08' WHERE NOT EXISTS (SELECT 1 FROM chart_of_accounts WHERE institution_id = 'HSBC');
