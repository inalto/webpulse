<?php

declare(strict_types=1);

namespace WebPulse\Store;

/**
 * SQL schemas (DATA-MODEL.md) as numbered migrations.
 */
final class Schema
{
    /** Latest domain database schema version. */
    public const VERSION = 1;

    /** Latest registry schema version. */
    public const REGISTRY_VERSION = 1;

    /** @return array<int,string[]> version => statements */
    public static function domainMigrations(): array
    {
        return [
            1 => [
                'CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT)',
                'CREATE TABLE IF NOT EXISTS daily (
                    day TEXT PRIMARY KEY,
                    visits INTEGER NOT NULL DEFAULT 0, visitors INTEGER NOT NULL DEFAULT 0,
                    pageviews INTEGER NOT NULL DEFAULT 0, hits INTEGER NOT NULL DEFAULT 0,
                    bytes INTEGER NOT NULL DEFAULT 0, bounces INTEGER NOT NULL DEFAULT 0,
                    duration INTEGER NOT NULL DEFAULT 0, downloads INTEGER NOT NULL DEFAULT 0,
                    errors_4xx INTEGER NOT NULL DEFAULT 0, errors_5xx INTEGER NOT NULL DEFAULT 0,
                    bot_hits INTEGER NOT NULL DEFAULT 0, bot_bytes INTEGER NOT NULL DEFAULT 0)',
                'CREATE TABLE IF NOT EXISTS hourly (
                    day TEXT NOT NULL, hour INTEGER NOT NULL,
                    visits INTEGER NOT NULL DEFAULT 0, pageviews INTEGER NOT NULL DEFAULT 0,
                    hits INTEGER NOT NULL DEFAULT 0, bytes INTEGER NOT NULL DEFAULT 0,
                    bot_hits INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (day, hour))',
                'CREATE TABLE IF NOT EXISTS dim_daily (
                    day TEXT NOT NULL, dim TEXT NOT NULL, key TEXT NOT NULL,
                    visits INTEGER NOT NULL DEFAULT 0, pageviews INTEGER NOT NULL DEFAULT 0,
                    hits INTEGER NOT NULL DEFAULT 0, bytes INTEGER NOT NULL DEFAULT 0,
                    bounces INTEGER NOT NULL DEFAULT 0, duration INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (day, dim, key)) WITHOUT ROWID',
                'CREATE INDEX IF NOT EXISTS dim_daily_dim_day ON dim_daily (dim, day)',
                'CREATE TABLE IF NOT EXISTS open_visits (
                    visitor TEXT PRIMARY KEY, visit_id INTEGER NOT NULL, day TEXT NOT NULL,
                    start_ts INTEGER NOT NULL, last_ts INTEGER NOT NULL, last_pv_ts INTEGER NOT NULL,
                    pageviews INTEGER NOT NULL, hits INTEGER NOT NULL, bytes INTEGER NOT NULL,
                    entry_page TEXT, exit_page TEXT, pages_seen TEXT NOT NULL, attrs TEXT NOT NULL)',
                'CREATE INDEX IF NOT EXISTS open_visits_last ON open_visits (last_ts)',
                'CREATE TABLE IF NOT EXISTS visits (
                    visit_id INTEGER PRIMARY KEY, visitor TEXT NOT NULL, day TEXT NOT NULL,
                    start_ts INTEGER NOT NULL, end_ts INTEGER NOT NULL, duration INTEGER NOT NULL,
                    pageviews INTEGER NOT NULL, hits INTEGER NOT NULL, bytes INTEGER NOT NULL,
                    entry_page TEXT, exit_page TEXT, channel TEXT, referrer_host TEXT, search_engine TEXT,
                    keyword TEXT, campaign TEXT, country TEXT, city TEXT, browser TEXT, browser_version TEXT,
                    os TEXT, os_version TEXT, device TEXT, ip TEXT)',
                'CREATE INDEX IF NOT EXISTS visits_day ON visits (day, start_ts)',
                'CREATE TABLE IF NOT EXISTS events (
                    id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, visit_id INTEGER, type TEXT NOT NULL,
                    path TEXT NOT NULL, status INTEGER NOT NULL, bytes INTEGER NOT NULL, referrer TEXT)',
                'CREATE INDEX IF NOT EXISTS events_ts ON events (ts)',
                'CREATE INDEX IF NOT EXISTS events_visit ON events (visit_id)',
                'CREATE TABLE IF NOT EXISTS visitor_day (
                    day TEXT NOT NULL, visitor TEXT NOT NULL, PRIMARY KEY (day, visitor)) WITHOUT ROWID',
                'CREATE TABLE IF NOT EXISTS checkpoints (
                    path TEXT PRIMARY KEY, inode INTEGER NOT NULL, offset INTEGER NOT NULL, size INTEGER NOT NULL,
                    first_line TEXT, last_ts INTEGER, lines INTEGER NOT NULL DEFAULT 0,
                    bad_lines INTEGER NOT NULL DEFAULT 0, ignored_lines INTEGER NOT NULL DEFAULT 0,
                    updated_at INTEGER NOT NULL)',
                // last_ts: newest line imported from the archive (lets a grown archive resume)
                'CREATE TABLE IF NOT EXISTS imported_archives (
                    path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL, lines INTEGER NOT NULL,
                    imported_at INTEGER NOT NULL, last_ts INTEGER)',
            ],
        ];
    }

    /** @return array<int,string[]> version => statements */
    public static function registryMigrations(): array
    {
        return [
            1 => [
                'CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT)',
                'CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)',
                'CREATE TABLE IF NOT EXISTS accounts (
                    user TEXT PRIMARY KEY, owner TEXT, enabled INTEGER, timezone TEXT,
                    first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, removed_at INTEGER)',
                'CREATE TABLE IF NOT EXISTS domains (
                    user TEXT NOT NULL, domain TEXT NOT NULL, enabled INTEGER, kind TEXT,
                    first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, removed_at INTEGER,
                    PRIMARY KEY (user, domain))',
                'CREATE TABLE IF NOT EXISTS domain_summary (
                    user TEXT NOT NULL, domain TEXT NOT NULL, db_bytes INTEGER NOT NULL DEFAULT 0,
                    last_collect_at INTEGER, last_line_ts INTEGER, lines_total INTEGER NOT NULL DEFAULT 0,
                    visits_today INTEGER NOT NULL DEFAULT 0, pageviews_today INTEGER NOT NULL DEFAULT 0,
                    visits_7d INTEGER NOT NULL DEFAULT 0, pageviews_7d INTEGER NOT NULL DEFAULT 0,
                    bytes_7d INTEGER NOT NULL DEFAULT 0, visits_30d INTEGER NOT NULL DEFAULT 0,
                    pageviews_30d INTEGER NOT NULL DEFAULT 0, bytes_30d INTEGER NOT NULL DEFAULT 0,
                    bot_hits_30d INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (user, domain))',
                'CREATE TABLE IF NOT EXISTS salts (day TEXT PRIMARY KEY, salt BLOB NOT NULL)',
                'CREATE TABLE IF NOT EXISTS jobs (
                    id INTEGER PRIMARY KEY, type TEXT NOT NULL, scope TEXT, started_at INTEGER NOT NULL,
                    finished_at INTEGER, status TEXT NOT NULL, lines INTEGER NOT NULL DEFAULT 0,
                    files INTEGER NOT NULL DEFAULT 0, domains INTEGER NOT NULL DEFAULT 0, message TEXT)',
            ],
        ];
    }
}
