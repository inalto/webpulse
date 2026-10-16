<?php

declare(strict_types=1);

namespace WebPulse\Store;

/**
 * Applies numbered migrations; the current version lives in
 * `meta.schema_version` (both domain databases and the registry).
 */
final class Migrator
{
    /** Reads meta.schema_version (0 when the database is empty). */
    public static function version(\PDO $pdo): int
    {
        try {
            $v = $pdo->query("SELECT value FROM meta WHERE key = 'schema_version'")->fetchColumn();
        } catch (\PDOException) {
            return 0;
        }
        return $v === false ? 0 : (int) $v;
    }

    /**
     * Brings $pdo up to the newest migration (writer side only).
     *
     * @param array<int,string[]> $migrations
     * @return int the resulting version
     * @throws SchemaTooNew when the database is newer than the code
     */
    public static function migrate(\PDO $pdo, array $migrations): int
    {
        ksort($migrations);
        $latest = (int) array_key_last($migrations);
        $current = self::version($pdo);
        if ($current > $latest) {
            throw new SchemaTooNew("database schema $current is newer than supported $latest");
        }
        if ($current === $latest) {
            return $current;
        }
        $pdo->exec('BEGIN IMMEDIATE');
        try {
            $current = self::version($pdo);
            foreach ($migrations as $v => $statements) {
                if ($v <= $current) {
                    continue;
                }
                foreach ($statements as $sql) {
                    $pdo->exec($sql);
                }
                $st = $pdo->prepare("INSERT INTO meta (key, value) VALUES ('schema_version', ?)
                    ON CONFLICT(key) DO UPDATE SET value = excluded.value");
                $st->execute([(string) $v]);
                $current = $v;
            }
            $pdo->exec('COMMIT');
        } catch (\Throwable $e) {
            $pdo->exec('ROLLBACK');
            throw $e;
        }
        return $current;
    }
}
