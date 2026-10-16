<?php

declare(strict_types=1);

namespace WebPulse\Store;

use WebPulse\Paths;
use WebPulse\Util\Fs;

/**
 * One SQLite database per domain: /var/webpulse/data/<user>/<domain>.db.
 *
 * Writer (root): journal_mode=DELETE so the account can read with read
 * permission only; file 0640 root:<user>, directory 0750 root:<user>.
 * Reader (dashboard): opened with SQLITE_OPEN_READONLY.
 */
final class DomainStore
{
    /** @var array<string,string|null>|null */
    private ?array $meta = null;

    private function __construct(
        private \PDO $pdo,
        private string $path,
        private string $user,
        private string $domain,
        private bool $readOnly,
    ) {
    }

    /**
     * Opens (and for writers creates/migrates) a domain database.
     *
     * @throws NotFound        read-only open of a missing database
     * @throws SchemaTooNew    read-only open of a database newer than this code
     * @throws \InvalidArgumentException invalid user/domain
     * @throws \RuntimeException|\PDOException on I/O errors
     */
    public static function open(Paths $paths, string $user, string $domain, bool $readOnly): self
    {
        $file = $paths->domainDb($user, $domain);
        if (is_link($file)) {
            throw new \RuntimeException("refusing symlinked database $file");
        }
        if ($readOnly) {
            if (!is_file($file)) {
                throw new NotFound("no data for $domain");
            }
            $pdo = new \PDO('sqlite:' . $file, null, null, [
                \PDO::ATTR_ERRMODE => \PDO::ERRMODE_EXCEPTION,
                \PDO::ATTR_TIMEOUT => 5,
                \PDO::SQLITE_ATTR_OPEN_FLAGS => \PDO::SQLITE_OPEN_READONLY,
            ]);
            $pdo->exec('PRAGMA busy_timeout = 5000');
            $pdo->exec('PRAGMA query_only = ON');
            $v = Migrator::version($pdo);
            if ($v > Schema::VERSION) {
                throw new SchemaTooNew("database of $domain was written by a newer WebPulse (schema $v)");
            }
            return new self($pdo, $file, $user, $domain, true);
        }

        self::prepareUserDir($paths, $user);
        $fresh = !is_file($file);
        $pdo = new \PDO('sqlite:' . $file, null, null, [
            \PDO::ATTR_ERRMODE => \PDO::ERRMODE_EXCEPTION,
            \PDO::ATTR_TIMEOUT => 5,
        ]);
        $pdo->exec('PRAGMA busy_timeout = 5000');
        $pdo->exec('PRAGMA journal_mode = DELETE');
        $pdo->exec('PRAGMA synchronous = NORMAL');
        $pdo->exec('PRAGMA foreign_keys = ON');
        Migrator::migrate($pdo, Schema::domainMigrations());
        if ($fresh) {
            @chmod($file, 0640);
            Fs::own($file, 'root', $user);
        }
        $store = new self($pdo, $file, $user, $domain, false);
        if ($fresh || $store->getMeta('domain') === null) {
            $store->setMetaMany([
                'user' => $user,
                'domain' => $domain,
                'created_at' => (string) \WebPulse\App::clock(),
                'next_visit_id' => $store->getMeta('next_visit_id') ?? '1',
                'total_lines' => $store->getMeta('total_lines') ?? '0',
            ]);
        }
        return $store;
    }

    /**
     * Creates <data_dir>/<user> (0750 root:<user>) and its `user/`
     * sub-directory (0700 <user>:<user>) when missing. Ownership is only
     * applied when running as root.
     */
    public static function prepareUserDir(Paths $paths, string $user): void
    {
        $data = $paths->dataDir();
        if (!is_dir($data)) {
            Fs::ensureDir($data, 0711, 'root', 'root');
        }
        $dir = $paths->userDir($user);
        if (!is_dir($dir) || Fs::isRoot()) {
            Fs::ensureDir($dir, 0750, 'root', $user);
        }
        $priv = $paths->userPrivateDir($user);
        if (is_link($priv)) {
            return; // never follow a user-planted symlink
        }
        if (!is_dir($priv)) {
            Fs::ensureDir($priv, 0700, $user, $user);
        }
    }

    /** The PDO (read-only when opened read-only). */
    public function pdo(): \PDO
    {
        return $this->pdo;
    }

    public function readOnly(): bool
    {
        return $this->readOnly;
    }

    public function path(): string
    {
        return $this->path;
    }

    public function user(): string
    {
        return $this->user;
    }

    public function domain(): string
    {
        return $this->domain;
    }

    /**
     * The meta table as an associative array.
     *
     * @return array<string,string|null>
     */
    public function meta(): array
    {
        if ($this->meta === null) {
            $this->meta = [];
            foreach ($this->pdo->query('SELECT key, value FROM meta')->fetchAll(\PDO::FETCH_NUM) as [$k, $v]) {
                $this->meta[(string) $k] = $v === null ? null : (string) $v;
            }
        }
        return $this->meta;
    }

    public function getMeta(string $key): ?string
    {
        return $this->meta()[$key] ?? null;
    }

    /** Sets one meta key (writer only; joins the current transaction if any). */
    public function setMeta(string $key, ?string $value): void
    {
        $this->setMetaMany([$key => $value]);
    }

    /** @param array<string,string|null> $values */
    public function setMetaMany(array $values): void
    {
        if ($this->readOnly) {
            throw new \LogicException('read-only store');
        }
        $st = $this->pdo->prepare('INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value');
        foreach ($values as $k => $v) {
            $st->execute([$k, $v]);
        }
        $this->meta = null;
    }

    /** Drops the cached meta (after external writes). */
    public function reloadMeta(): void
    {
        $this->meta = null;
    }

    /** Closes the connection (the object must not be used afterwards). */
    public function close(): void
    {
        unset($this->pdo);
    }
}
