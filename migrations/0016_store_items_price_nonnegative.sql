-- Enforce non-negative price on store_items (#144)
ALTER TABLE store_items
    ADD CONSTRAINT store_items_price_nonnegative CHECK (price >= 0) NOT VALID;

ALTER TABLE store_items VALIDATE CONSTRAINT store_items_price_nonnegative;
