-- WishSync M1（MVP）遷移：取自 docs/04 v0.4 最終 schema 的 MVP 子集
-- 不含 relief（P2-D，PostGIS 屆時才建）與點數眾籌（P2-A）欄位；見 docs/04「MVP 遷移範圍」

CREATE OR REPLACE FUNCTION uuid_v7() RETURNS uuid
LANGUAGE sql VOLATILE AS $$
  SELECT encode(
    set_bit(
      set_bit(
        overlay(uuid_send(gen_random_uuid())
                placing substring(int8send((extract(epoch FROM clock_timestamp()) * 1000)::bigint) FROM 3)
                FROM 1 FOR 6),
        52, 1),
      53, 1),
    'hex')::uuid
$$;

CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.updated_at := now();
  RETURN NEW;
END $$;

CREATE TYPE wishlist_type        AS ENUM ('personal', 'registry', 'relief');
CREATE TYPE wishlist_status      AS ENUM ('draft', 'active', 'closed', 'archived');
CREATE TYPE visibility           AS ENUM ('link', 'private', 'public');
CREATE TYPE funding_mode         AS ENUM ('quantity', 'crowdfund');
CREATE TYPE item_priority        AS ENUM ('high', 'medium', 'low');
CREATE TYPE claim_status         AS ENUM ('reserved', 'purchased', 'delivered', 'cancelled', 'expired');
CREATE TYPE auth_provider        AS ENUM ('email', 'line', 'google');
CREATE TYPE moderation_status    AS ENUM ('ok', 'hidden');
CREATE TYPE report_reason        AS ENUM ('scam', 'inappropriate', 'copyright', 'personal_info', 'other');
CREATE TYPE report_status        AS ENUM ('open', 'actioned', 'dismissed');
CREATE TYPE image_status         AS ENUM ('none', 'pending', 'ready', 'rejected');
CREATE TYPE actor_type           AS ENUM ('user', 'guest', 'system', 'staff');
CREATE TYPE notification_channel AS ENUM ('email', 'line');
CREATE TYPE notification_status  AS ENUM ('pending', 'sent', 'failed', 'cancelled');

CREATE TABLE users (
  id            uuid PRIMARY KEY DEFAULT uuid_v7(),
  display_name  text        NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 50),
  email         text        UNIQUE CHECK (email = lower(email)),
  avatar_key    text,
  locale        text        NOT NULL DEFAULT 'zh-TW',
  is_staff      boolean     NOT NULL DEFAULT false,   -- 營運人員（MVP 起：/admin 後台；P2：人工代購）
  notification_prefs jsonb  NOT NULL DEFAULT '{"email_claims": true}'::jsonb,   -- 通知設定（PATCH /me）；MVP 鍵：email_claims
  anonymized_at timestamptz,                              -- 帳號刪除去識別化時間
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  deleted_at    timestamptz
);

CREATE TABLE auth_identities (
  id            uuid PRIMARY KEY DEFAULT uuid_v7(),
  user_id       uuid          NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  provider      auth_provider NOT NULL,
  provider_uid  text          NOT NULL,           -- email: 小寫 email；line/google: sub
  email         text,
  created_at    timestamptz   NOT NULL DEFAULT now(),
  UNIQUE (provider, provider_uid)
);

CREATE INDEX auth_identities_user_idx ON auth_identities (user_id);
CREATE TABLE otp_challenges (
  id            uuid PRIMARY KEY DEFAULT uuid_v7(),
  email         text        NOT NULL CHECK (email = lower(email)),
  code_hash     bytea       NOT NULL,             -- SHA-256(code || server_pepper)
  attempts      smallint    NOT NULL DEFAULT 0,
  expires_at    timestamptz NOT NULL,
  consumed_at   timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX otp_challenges_email_idx ON otp_challenges (email, created_at DESC);
CREATE TABLE sessions (
  id            uuid PRIMARY KEY DEFAULT uuid_v7(),
  user_id       uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_hash    bytea       NOT NULL UNIQUE,      -- SHA-256(opaque token)
  user_agent    text,
  ip            inet,
  expires_at    timestamptz NOT NULL,
  last_seen_at  timestamptz NOT NULL DEFAULT now(),
  revoked_at    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX sessions_user_idx ON sessions (user_id);
CREATE INDEX sessions_expires_idx ON sessions (expires_at) WHERE revoked_at IS NULL;
CREATE TABLE guests (
  id               uuid PRIMARY KEY DEFAULT uuid_v7(),
  guest_token_hash bytea       NOT NULL UNIQUE,   -- SHA-256(32 bytes token)
  display_name     text        NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 30),
  contact          text        CHECK (char_length(contact) <= 100),
  email            text        CHECK (email = lower(email)),   -- 選填：認領確認信與「管理我的認領」連結（D39）
  deleted_at       timestamptz,                                  -- DELETE /guest/me：清除暱稱 / 聯絡方式 / email，claims 保留
  created_at       timestamptz NOT NULL DEFAULT now(),
  updated_at       timestamptz NOT NULL DEFAULT now(),
  last_seen_at     timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE guest_recovery_tokens (
  id          uuid PRIMARY KEY DEFAULT uuid_v7(),
  guest_id    uuid        NOT NULL REFERENCES guests(id) ON DELETE CASCADE,
  token_hash  bytea       NOT NULL UNIQUE,           -- SHA-256(權杖)；權杖明文只出現在信件連結 /me/claims#r={token}
  expires_at  timestamptz NOT NULL,                  -- 建立後 30 天
  used_at     timestamptz,
  created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX guest_recovery_tokens_guest_idx ON guest_recovery_tokens (guest_id, created_at DESC);
CREATE TABLE wishlists (
  id                 uuid PRIMARY KEY DEFAULT uuid_v7(),
  owner_id           uuid            NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  type               wishlist_type   NOT NULL,
  status             wishlist_status NOT NULL DEFAULT 'draft',
  visibility         visibility      NOT NULL DEFAULT 'link',
  slug               char(10)        NOT NULL,
  title              text            NOT NULL CHECK (char_length(title) BETWEEN 1 AND 100),
  description        text,
  cover_image_key    text,
  cover_image_status image_status    NOT NULL DEFAULT 'none',
  event_date         date,
  show_claimer_names boolean         NOT NULL DEFAULT false,
  surprise_mode      boolean         NOT NULL DEFAULT false,
  moderation_status  moderation_status NOT NULL DEFAULT 'ok',   -- hidden → 公開頁 410 WISHLIST_REMOVED
  moderation_reason  text,
  moderated_at       timestamptz,
  moderated_by       uuid            REFERENCES users(id) ON DELETE SET NULL,
  claim_ttl_hours    integer         CHECK (claim_ttl_hours > 0),   -- NULL = 認領不逾期；relief 建立時預設 48
  closed_at          timestamptz,
  created_at         timestamptz     NOT NULL DEFAULT now(),
  updated_at         timestamptz     NOT NULL DEFAULT now(),
  deleted_at         timestamptz,
  CONSTRAINT wishlists_slug_format CHECK (slug ~ '^[0-9A-Za-z]{10}$'),
  CONSTRAINT wishlists_surprise_not_relief CHECK (NOT surprise_mode OR type <> 'relief'),
  CONSTRAINT wishlists_public_only_relief CHECK (visibility <> 'public' OR type = 'relief'),
  CONSTRAINT wishlists_hidden_has_reason CHECK (moderation_status = 'ok' OR moderation_reason IS NOT NULL),
  CONSTRAINT wishlists_relief_no_names CHECK (type <> 'relief' OR show_claimer_names = false)
);

CREATE UNIQUE INDEX wishlists_slug_key ON wishlists (slug);
CREATE INDEX wishlists_owner_idx ON wishlists (owner_id, status) WHERE deleted_at IS NULL;
CREATE INDEX wishlists_hidden_idx ON wishlists (moderated_at DESC) WHERE moderation_status = 'hidden';
CREATE TABLE content_reports (
  id                uuid PRIMARY KEY DEFAULT uuid_v7(),
  wishlist_id       uuid          NOT NULL REFERENCES wishlists(id) ON DELETE CASCADE,
  item_id           uuid,         -- 選填：被檢舉的品項（不設 FK，品項軟刪除後報告仍保留）
  reason            report_reason NOT NULL,
  detail            text          CHECK (char_length(detail) <= 1000),
  reporter_guest_id uuid          REFERENCES guests(id) ON DELETE SET NULL,
  reporter_user_id  uuid          REFERENCES users(id) ON DELETE SET NULL,
  status            report_status NOT NULL DEFAULT 'open',
  handled_by        uuid          REFERENCES users(id) ON DELETE SET NULL,
  handled_at        timestamptz,
  created_at        timestamptz   NOT NULL DEFAULT now(),
  CONSTRAINT reports_single_reporter CHECK (reporter_guest_id IS NULL OR reporter_user_id IS NULL),
  CONSTRAINT reports_handled_consistent CHECK ((status = 'open') = (handled_at IS NULL))
);

CREATE INDEX content_reports_queue_idx ON content_reports (status, created_at DESC);
CREATE INDEX content_reports_wishlist_idx ON content_reports (wishlist_id, created_at DESC);
CREATE TABLE wishlist_items (
  id               uuid PRIMARY KEY DEFAULT uuid_v7(),
  wishlist_id      uuid          NOT NULL REFERENCES wishlists(id) ON DELETE CASCADE,
  title            text          NOT NULL CHECK (char_length(title) BETWEEN 1 AND 120),
  description      text,
  brand            text,
  spec             text,
  image_key        text,
  image_status     image_status  NOT NULL DEFAULT 'none',   -- none | pending（已上傳待處理）| ready | rejected
  product_url      text,
  unit_price_amount bigint       CHECK (unit_price_amount >= 0),
  funding_mode     funding_mode  NOT NULL DEFAULT 'quantity',
  priority         item_priority NOT NULL DEFAULT 'medium',
  qty_needed       integer       NOT NULL DEFAULT 1,
  qty_claimed      integer       NOT NULL DEFAULT 0,   -- 快取
  sort_order       integer       NOT NULL DEFAULT 0,
  created_at       timestamptz   NOT NULL DEFAULT now(),
  updated_at       timestamptz   NOT NULL DEFAULT now(),
  deleted_at       timestamptz,
  CONSTRAINT items_image_status_key    CHECK ((image_status <> 'none' OR image_key IS NULL) AND (image_status NOT IN ('pending', 'ready') OR image_key IS NOT NULL)),
  CONSTRAINT items_qty_needed_pos      CHECK (qty_needed > 0),
  CONSTRAINT items_qty_claimed_range   CHECK (qty_claimed >= 0 AND qty_claimed <= qty_needed),
  CONSTRAINT items_mvp_quantity_only   CHECK (funding_mode = 'quantity')   -- P2-A 遷移時放寬
);
CREATE INDEX items_wishlist_idx ON wishlist_items (wishlist_id, sort_order) WHERE deleted_at IS NULL;
CREATE INDEX items_open_idx ON wishlist_items (wishlist_id) WHERE deleted_at IS NULL AND qty_claimed < qty_needed;
CREATE TABLE claims (
  id            uuid PRIMARY KEY DEFAULT uuid_v7(),
  item_id       uuid         NOT NULL REFERENCES wishlist_items(id) ON DELETE CASCADE,
  guest_id      uuid         REFERENCES guests(id) ON DELETE SET NULL,
  user_id       uuid         REFERENCES users(id) ON DELETE SET NULL,
  claimer_name  text         NOT NULL,                 -- 認領當下暱稱快照
  qty           integer      NOT NULL CHECK (qty > 0),
  status        claim_status NOT NULL DEFAULT 'reserved',
  note          text         CHECK (char_length(note) <= 200),
  expires_at    timestamptz,
  purchased_at  timestamptz,
  delivered_at  timestamptz,
  cancelled_at  timestamptz,
  created_at    timestamptz  NOT NULL DEFAULT now(),
  updated_at    timestamptz  NOT NULL DEFAULT now(),
  -- 帳號刪除 / 訪客清除後兩者皆可為 NULL，故只禁止「兩者同時有值」
  CONSTRAINT claims_single_actor CHECK (guest_id IS NULL OR user_id IS NULL)
);

CREATE INDEX claims_item_idx ON claims (item_id);
-- 同一 guest / user 對同一品項只能有一筆有效認領（改數量請用 PATCH）
CREATE UNIQUE INDEX claims_item_guest_active_uk ON claims (item_id, guest_id)
  WHERE guest_id IS NOT NULL AND status IN ('reserved', 'purchased', 'delivered');
CREATE UNIQUE INDEX claims_item_user_active_uk ON claims (item_id, user_id)
  WHERE user_id IS NOT NULL AND status IN ('reserved', 'purchased', 'delivered');
CREATE INDEX claims_item_active_idx ON claims (item_id) WHERE status IN ('reserved', 'purchased', 'delivered');
CREATE INDEX claims_guest_idx ON claims (guest_id) WHERE guest_id IS NOT NULL;
CREATE INDEX claims_user_idx ON claims (user_id) WHERE user_id IS NOT NULL;
CREATE INDEX claims_expiry_idx ON claims (expires_at) WHERE status = 'reserved' AND expires_at IS NOT NULL;
CREATE TABLE idempotency_keys (
  key             uuid        NOT NULL,
  scope           text        NOT NULL,        -- 例：guest:<guest_id>:POST /items/{id}/claims
  request_hash    bytea       NOT NULL,        -- SHA-256(method + path + body)
  response_status integer,                     -- NULL = 處理中
  response_body   jsonb,
  created_at      timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (scope, key)
);

CREATE TABLE audit_logs (
  id          uuid PRIMARY KEY DEFAULT uuid_v7(),
  actor_type  actor_type  NOT NULL,
  actor_id    uuid,
  action      text        NOT NULL,            -- 例：claim.create / wishlist.update
  entity      text        NOT NULL,            -- 例：claims / wishlists
  entity_id   uuid,
  diff        jsonb,
  ip          inet,
  created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE system_flags (
  key         text PRIMARY KEY,
  value       jsonb       NOT NULL,
  updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE notifications (
  id            uuid PRIMARY KEY DEFAULT uuid_v7(),
  user_id       uuid                 REFERENCES users(id) ON DELETE CASCADE,
  guest_id      uuid                 REFERENCES guests(id) ON DELETE CASCADE,   -- 訪客認領確認信（D39）
  channel       notification_channel NOT NULL,
  kind          text                 NOT NULL,       -- claim.created / claim.digest / claim.confirmation / event.reminder
  payload       jsonb                NOT NULL DEFAULT '{}'::jsonb,
  status        notification_status  NOT NULL DEFAULT 'pending',
  scheduled_at  timestamptz          NOT NULL DEFAULT now(),
  sent_at       timestamptz,
  attempts      smallint             NOT NULL DEFAULT 0,
  last_error    text,
  created_at    timestamptz          NOT NULL DEFAULT now(),
  CONSTRAINT notifications_has_recipient CHECK (user_id IS NOT NULL OR guest_id IS NOT NULL)
);

INSERT INTO system_flags (key, value) VALUES ('read_only', 'false'::jsonb);
CREATE INDEX idempotency_keys_created_idx ON idempotency_keys (created_at);  -- 24h 清除 job
CREATE INDEX audit_logs_entity_idx ON audit_logs (entity, entity_id, created_at DESC);
CREATE INDEX audit_logs_actor_idx ON audit_logs (actor_type, actor_id, created_at DESC);
CREATE UNIQUE INDEX notifications_digest_uk ON notifications (user_id, kind, (payload->>'wishlist_id'))
  WHERE kind = 'claim.digest' AND status = 'pending';
CREATE INDEX notifications_due_idx ON notifications (scheduled_at) WHERE status = 'pending';
CREATE INDEX notifications_user_idx ON notifications (user_id, created_at DESC);
CREATE TRIGGER users_set_updated_at          BEFORE UPDATE ON users          FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER guests_set_updated_at         BEFORE UPDATE ON guests         FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER wishlists_set_updated_at      BEFORE UPDATE ON wishlists      FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER wishlist_items_set_updated_at BEFORE UPDATE ON wishlist_items FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER claims_set_updated_at         BEFORE UPDATE ON claims         FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER system_flags_set_updated_at   BEFORE UPDATE ON system_flags   FOR EACH ROW EXECUTE FUNCTION set_updated_at();
