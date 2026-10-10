-- P2-A 點數眾籌（docs/04 第 3 章 P2-A 子集）
-- 範圍裁剪：無儲值 / 金流 / 點數批次（point_lots）/ 自動儲值 / 現金退款 / 商品目錄；點數由營運人工發放（ledger grant）。
-- 履約只有 concierge（營運人工代購），故 fulfillment_type 只允許 concierge。
CREATE TYPE contribution_status AS ENUM ('pledged', 'captured', 'released', 'reallocated');
CREATE TYPE funding_status      AS ENUM ('open', 'funded', 'expired', 'fulfilled');
CREATE TYPE fulfillment_type    AS ENUM ('catalog', 'concierge');
-- grant 營運發放；pledge 認捐扣點；release 整筆退回（撤回 / 逾期 / 購買失敗 / 品項刪除）；refund 實際花費較低的差額退回；adjustment 人工更正
CREATE TYPE ledger_entry_type   AS ENUM ('grant', 'pledge', 'release', 'refund', 'adjustment');
CREATE TYPE wallet_status       AS ENUM ('active', 'frozen');
CREATE TYPE order_status        AS ENUM ('pending', 'placed', 'shipped', 'delivered', 'failed', 'cancelled');

-- ---------- wishlist_items：眾籌欄位 ----------
ALTER TABLE wishlist_items DROP CONSTRAINT items_mvp_quantity_only;
ALTER TABLE wishlist_items
  ADD COLUMN target_points         bigint,                              -- = 商品價 + 運費 + 服務費 / 緩衝
  ADD COLUMN pledged_points        bigint NOT NULL DEFAULT 0,           -- 快取：status in (pledged, captured) 的 contributions.points 加總
  ADD COLUMN funding_status        funding_status,                      -- quantity 為 NULL；crowdfund 建立時 'open'
  ADD COLUMN funding_deadline      timestamptz,
  ADD COLUMN fulfillment_type      fulfillment_type,
  ADD COLUMN price_snapshot_amount bigint CHECK (price_snapshot_amount >= 0),
  ADD COLUMN expired_at            timestamptz,                         -- 轉為 expired 的時間；7 天選擇期自此起算
  ADD CONSTRAINT items_points_nonneg       CHECK (pledged_points >= 0),
  ADD CONSTRAINT items_crowdfund_shape     CHECK (funding_mode <> 'crowdfund' OR (
      qty_needed = 1 AND target_points IS NOT NULL AND target_points > 0
      AND fulfillment_type IS NOT NULL AND funding_status IS NOT NULL AND funding_deadline IS NOT NULL)),
  ADD CONSTRAINT items_concierge_only      CHECK (fulfillment_type IS DISTINCT FROM 'catalog'),   -- 商品目錄未實作
  ADD CONSTRAINT items_quantity_no_funding CHECK (funding_mode <> 'quantity' OR (
      target_points IS NULL AND pledged_points = 0 AND funding_status IS NULL AND funding_deadline IS NULL
      AND fulfillment_type IS NULL AND price_snapshot_amount IS NULL AND expired_at IS NULL)),
  ADD CONSTRAINT items_expired_has_time    CHECK ((funding_status = 'expired') = (expired_at IS NOT NULL) OR funding_status IS NULL),
  ADD CONSTRAINT items_crowdfund_no_claims CHECK (funding_mode <> 'crowdfund' OR qty_claimed = 0),
  -- 硬上限：絕不超額
  ADD CONSTRAINT items_pledged_le_target   CHECK (target_points IS NULL OR pledged_points <= target_points),
  ADD CONSTRAINT items_funded_is_full      CHECK (funding_status IS NULL OR funding_status NOT IN ('funded', 'fulfilled') OR pledged_points = target_points);
CREATE INDEX items_funding_deadline_idx ON wishlist_items (funding_deadline) WHERE funding_status IN ('open', 'expired');

-- ---------- point_wallets / point_ledger ----------
-- 1 點 = NT$1。點數只能投入心願品項；不可轉讓、不可提領給受捐者。balance 與 ledger 加總一致（對帳）。
CREATE TABLE point_wallets (
  id          uuid PRIMARY KEY DEFAULT uuid_v7(),
  user_id     uuid          NOT NULL UNIQUE REFERENCES users(id) ON DELETE RESTRICT,
  balance     bigint        NOT NULL DEFAULT 0,
  status      wallet_status NOT NULL DEFAULT 'active',   -- frozen → 認捐回 403 WALLET_FROZEN
  created_at  timestamptz   NOT NULL DEFAULT now(),
  updated_at  timestamptz   NOT NULL DEFAULT now(),
  CONSTRAINT point_wallets_balance_nonneg CHECK (balance >= 0)
);

CREATE TABLE point_ledger (
  id             uuid PRIMARY KEY DEFAULT uuid_v7(),
  seq            bigint             GENERATED ALWAYS AS IDENTITY,   -- 嚴格遞增，同毫秒排序與對帳
  wallet_id      uuid               NOT NULL REFERENCES point_wallets(id) ON DELETE RESTRICT,
  delta          bigint             NOT NULL CHECK (delta <> 0),
  balance_after  bigint             NOT NULL CHECK (balance_after >= 0),
  entry_type     ledger_entry_type  NOT NULL,
  ref_type       text,              -- contribution | manual
  ref_id         uuid,
  note           text,              -- grant / adjustment 的原因
  actor_id       uuid               REFERENCES users(id) ON DELETE SET NULL,   -- grant / adjustment 的營運人員
  created_at     timestamptz        NOT NULL DEFAULT now()
);
CREATE INDEX point_ledger_wallet_idx ON point_ledger (wallet_id, seq DESC);
CREATE UNIQUE INDEX point_ledger_seq_uk ON point_ledger (seq);
-- 防重複入帳：同一筆認捐的 pledge / release / refund 各只能記一次
CREATE UNIQUE INDEX point_ledger_ref_uk ON point_ledger (wallet_id, entry_type, ref_type, ref_id) WHERE ref_type = 'contribution';

CREATE OR REPLACE FUNCTION point_ledger_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'point_ledger is append-only' USING ERRCODE = 'restrict_violation';
END $$;
CREATE TRIGGER point_ledger_no_update BEFORE UPDATE OR DELETE ON point_ledger
  FOR EACH ROW EXECUTE FUNCTION point_ledger_immutable();

-- ---------- contributions（點數認捐；僅登入使用者）----------
CREATE TABLE contributions (
  id                      uuid PRIMARY KEY DEFAULT uuid_v7(),
  item_id                 uuid                NOT NULL REFERENCES wishlist_items(id) ON DELETE RESTRICT,
  wishlist_id             uuid                NOT NULL REFERENCES wishlists(id) ON DELETE RESTRICT,
  user_id                 uuid                NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  wallet_id               uuid                NOT NULL REFERENCES point_wallets(id) ON DELETE RESTRICT,
  donor_name              text                NOT NULL,           -- 認捐當下暱稱快照（同 claims.claimer_name）
  points                  bigint              NOT NULL CHECK (points > 0),
  refunded_points         bigint              NOT NULL DEFAULT 0, -- 實際花費 = points - refunded_points
  status                  contribution_status NOT NULL DEFAULT 'pledged',
  message                 text                CHECK (char_length(message) <= 200),
  is_anonymous            boolean             NOT NULL DEFAULT false,
  reallocated_to_item_id  uuid                REFERENCES wishlist_items(id) ON DELETE RESTRICT,
  captured_at             timestamptz,
  released_at             timestamptz,
  created_at              timestamptz         NOT NULL DEFAULT now(),
  updated_at              timestamptz         NOT NULL DEFAULT now(),
  CONSTRAINT contributions_refunded_range CHECK (refunded_points >= 0 AND refunded_points <= points),
  CONSTRAINT contributions_realloc_consistent CHECK ((status = 'reallocated') = (reallocated_to_item_id IS NOT NULL)),
  CONSTRAINT contributions_captured_has_time CHECK (status <> 'captured' OR captured_at IS NOT NULL),
  CONSTRAINT contributions_released_has_time CHECK (status <> 'released' OR released_at IS NOT NULL)
);
CREATE INDEX contributions_item_idx ON contributions (item_id);
CREATE INDEX contributions_item_active_idx ON contributions (item_id) WHERE status IN ('pledged', 'captured');
CREATE INDEX contributions_user_idx ON contributions (user_id, created_at DESC);
CREATE INDEX contributions_wallet_idx ON contributions (wallet_id);

-- ---------- shipping_addresses（欄位級加密，見 src/sealed.rs）----------
CREATE TABLE shipping_addresses (
  id                  uuid PRIMARY KEY DEFAULT uuid_v7(),
  wishlist_id         uuid        NOT NULL UNIQUE REFERENCES wishlists(id) ON DELETE CASCADE,
  recipient_name_enc  bytea       NOT NULL,
  phone_enc           bytea       NOT NULL,
  address_enc         bytea       NOT NULL,
  key_version         smallint    NOT NULL DEFAULT 1,
  created_at          timestamptz NOT NULL DEFAULT now(),
  updated_at          timestamptz NOT NULL DEFAULT now()
);

-- ---------- purchase_orders（達標後的採購單；concierge 人工代購）----------
CREATE TABLE purchase_orders (
  id                        uuid PRIMARY KEY DEFAULT uuid_v7(),
  item_id                   uuid             NOT NULL UNIQUE REFERENCES wishlist_items(id) ON DELETE RESTRICT,  -- 失敗後重新達標時重用同一列
  fulfillment_type          fulfillment_type NOT NULL,
  merchant_order_id         text,
  amount                    bigint           NOT NULL CHECK (amount > 0),   -- 建立時 = target_points；placed 時更新為實際花費（<= target）
  status                    order_status     NOT NULL DEFAULT 'pending',
  tracking_no               text,
  shipping_address_snapshot bytea            NOT NULL,                      -- 建單當下收件資訊快照（sealed JSON）
  failure_reason            text,
  operator_user_id          uuid             REFERENCES users(id) ON DELETE SET NULL,
  placed_at                 timestamptz,
  shipped_at                timestamptz,
  delivered_at              timestamptz,
  created_at                timestamptz      NOT NULL DEFAULT now(),
  updated_at                timestamptz      NOT NULL DEFAULT now(),
  CONSTRAINT orders_placed_has_ref CHECK (status NOT IN ('placed', 'shipped', 'delivered') OR merchant_order_id IS NOT NULL),
  CONSTRAINT orders_failed_has_reason CHECK (status NOT IN ('failed', 'cancelled') OR failure_reason IS NOT NULL)
);
CREATE INDEX purchase_orders_queue_idx ON purchase_orders (status, created_at) WHERE status IN ('pending', 'placed', 'shipped');

CREATE TRIGGER point_wallets_set_updated_at      BEFORE UPDATE ON point_wallets      FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER contributions_set_updated_at      BEFORE UPDATE ON contributions      FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER shipping_addresses_set_updated_at BEFORE UPDATE ON shipping_addresses FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER purchase_orders_set_updated_at    BEFORE UPDATE ON purchase_orders    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
