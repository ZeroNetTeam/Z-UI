-- A plan a user moves on to when it runs out of data or time, instead of
-- being cut off: its new data limit (0 for none), its days (0 for no date),
-- and whether the data it had left carries over. No plan while the limit is
-- NULL.
ALTER TABLE users ADD COLUMN next_data_limit BIGINT;
ALTER TABLE users ADD COLUMN next_expire_days BIGINT NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN next_carry_over BIGINT NOT NULL DEFAULT 0;
