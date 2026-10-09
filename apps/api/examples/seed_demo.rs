//! 本機測試帳號：cargo run --example seed_demo
//! demo@demo.com / demo123（只有 dev；密碼 7 碼，不符註冊規則，所以直接寫 DB），並附一份示範清單。可重複執行。
use argon2::{password_hash::{rand_core::OsRng, SaltString}, Argon2, PasswordHasher};

#[tokio::main]
async fn main() {
    assert!(std::env::var("APP_ENV").unwrap_or("dev".into()) == "dev", "seed_demo 只能在 dev 執行");
    let pool = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL")).await.unwrap();
    let hash = Argon2::default().hash_password(b"demo123", &SaltString::generate(&mut OsRng)).unwrap().to_string();
    let uid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (display_name, email) VALUES ('Demo', 'demo@demo.com')
         ON CONFLICT (email) DO UPDATE SET deleted_at = NULL RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO auth_identities (user_id, provider, provider_uid, email, password_hash)
         VALUES ($1, 'email', 'demo@demo.com', 'demo@demo.com', $2)
         ON CONFLICT (provider, provider_uid) DO UPDATE SET password_hash = EXCLUDED.password_hash")
        .bind(uid).bind(&hash).execute(&pool).await.unwrap();
    let wid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO wishlists (owner_id, type, status, slug, title, description, event_date)
         VALUES ($1, 'registry', 'active', 'DemoWish01', '小愛的待產清單', '預產期 12 月，謝謝大家的心意', '2026-12-20')
         ON CONFLICT (slug) DO UPDATE SET title = EXCLUDED.title RETURNING id").bind(uid).fetch_one(&pool).await.unwrap();
    if sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wishlist_items WHERE wishlist_id = $1").bind(wid).fetch_one(&pool).await.unwrap() == 0 {
        for (i, (t, p, n, c)) in [("Combi 嬰兒推車", "high", 1, 0), ("NB 尿布 2 包", "medium", 2, 1), ("奶粉 1 號 800g", "low", 2, 2)].iter().enumerate() {
            sqlx::query("INSERT INTO wishlist_items (wishlist_id, title, priority, qty_needed, qty_claimed, sort_order) VALUES ($1, $2, $3::item_priority, $4, $5, $6)")
                .bind(wid).bind(t).bind(p).bind(*n as i32).bind(*c as i32).bind(i as i32).execute(&pool).await.unwrap();
        }
    }
    println!("OK  demo@demo.com / demo123  →  /s/DemoWish01");
}
