-- 社群可見性：好友、個人頁、清單有限度公開（好友 / 指定名單 / 存取密碼）、捐助（認領）公開設定。
-- 清單 visibility：
--   public   任何人可看，並列在擁有者個人頁 /u/{handle}
--   link     知道連結即可看，不列在個人頁（既有行為）
--   friends  僅擁有者的好友（需登入）
--   selected 僅 wishlist_allowed_users 名單內的使用者（需登入）
--   password 知道連結＋存取密碼
--   private  僅擁有者
-- 注意：ADD VALUE 在同一交易內不可使用新值，下方 CHECK 用 ::text 比較避開。
ALTER TYPE visibility ADD VALUE IF NOT EXISTS 'friends';
ALTER TYPE visibility ADD VALUE IF NOT EXISTS 'selected';
ALTER TYPE visibility ADD VALUE IF NOT EXISTS 'password';

-- public 不再限 relief：個人清單也可公開到個人頁
ALTER TABLE wishlists DROP CONSTRAINT wishlists_public_only_relief;
ALTER TABLE wishlists
  ADD COLUMN access_password_hash text,   -- argon2；僅 visibility=password 時有值
  ADD CONSTRAINT wishlists_password_has_hash CHECK ((visibility::text = 'password') = (access_password_hash IS NOT NULL));

CREATE TABLE wishlist_allowed_users (
  wishlist_id uuid        NOT NULL REFERENCES wishlists(id) ON DELETE CASCADE,
  user_id     uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (wishlist_id, user_id)
);
CREATE INDEX wishlist_allowed_users_user_idx ON wishlist_allowed_users (user_id);

-- 捐助（認領）公開層級；也是使用者預設值
CREATE TYPE share_level AS ENUM ('public', 'friends', 'private');

ALTER TABLE users
  ADD COLUMN handle text CHECK (handle ~ '^[a-z0-9_]{3,30}$'),   -- 個人頁 /u/{handle}；NULL = 尚未設定（無個人頁）
  ADD COLUMN default_claim_visibility share_level NOT NULL DEFAULT 'private';
CREATE UNIQUE INDEX users_handle_key ON users (handle) WHERE handle IS NOT NULL;

-- NULL = 訪客認領（無帳號，沿用清單 show_claimer_names 舊行為，不出現在任何個人頁）
ALTER TABLE claims ADD COLUMN visibility share_level;
UPDATE claims SET visibility = 'private' WHERE user_id IS NOT NULL;
ALTER TABLE claims ADD CONSTRAINT claims_visibility_user CHECK ((user_id IS NULL) OR (visibility IS NOT NULL));
-- 登入者認領未指定 visibility → 套用帳號預設（每筆可在建立/PATCH 時覆寫）
CREATE OR REPLACE FUNCTION claims_default_visibility() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.user_id IS NOT NULL AND NEW.visibility IS NULL THEN
    SELECT default_claim_visibility INTO NEW.visibility FROM users WHERE id = NEW.user_id;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER claims_default_visibility BEFORE INSERT OR UPDATE OF user_id ON claims
  FOR EACH ROW EXECUTE FUNCTION claims_default_visibility();

-- 好友：無向，一列一對，user_a < user_b
CREATE TABLE friendships (
  user_a     uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  user_b     uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_a, user_b),
  CONSTRAINT friendships_ordered CHECK (user_a < user_b)
);
CREATE INDEX friendships_b_idx ON friendships (user_b);

-- 好友申請（以 email 或 handle 指定對象）；接受後刪除並寫入 friendships
CREATE TABLE friend_requests (
  id         uuid PRIMARY KEY DEFAULT uuid_v7(),
  from_user  uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  to_user    uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT friend_requests_not_self CHECK (from_user <> to_user),
  UNIQUE (from_user, to_user)
);
CREATE INDEX friend_requests_to_idx ON friend_requests (to_user, created_at DESC);

-- 好友邀請連結 /invite/{token}（可 QR）：點開並登入即互為好友；可多次使用直到過期或撤銷
CREATE TABLE friend_invites (
  id         uuid PRIMARY KEY DEFAULT uuid_v7(),
  user_id    uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_hash bytea       NOT NULL UNIQUE,   -- SHA-256(token)
  expires_at timestamptz NOT NULL,
  revoked_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX friend_invites_user_idx ON friend_invites (user_id, created_at DESC);
