<?php

declare(strict_types=1);

use WebPulse\Log\LogSource;
use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\T;

$dir = static function (): string {
    $d = FakeRoot::repo() . '/tests/tmp/logsource';
    @mkdir($d, 0755, true);
    return $d;
};
$read = static function (LogSource $s): array {
    $out = [];
    while (($l = $s->next()) !== null) {
        $out[] = $l;
    }
    return $out;
};
$cp = static fn (LogSource $s): array => ['inode' => $s->inode(), 'offset' => $s->offset(), 'size' => $s->size(), 'first_line' => $s->fingerprint()];

return [
    'reads complete lines only, resumes at checkpoint' => static function () use ($dir, $read, $cp): void {
        $f = $dir() . '/a.log';
        file_put_contents($f, "one\ntwo\r\nthree-partial");
        $s = new LogSource($f);
        $s->open();
        T::eq(['one', 'two'], $read($s));
        T::eq(9, $s->offset());
        $c = $cp($s);
        $s->close();
        file_put_contents($f, "-done\nfour\n", FILE_APPEND);
        $s2 = new LogSource($f, false, $c);
        $s2->open();
        T::null($s2->resetReason());
        T::eq(['three-partial-done', 'four'], $read($s2));
    },
    'truncation restarts from zero' => static function () use ($dir, $read, $cp): void {
        $f = $dir() . '/t.log';
        file_put_contents($f, str_repeat("line\n", 100));
        $s = new LogSource($f);
        $s->open();
        $read($s);
        $c = $cp($s);
        $s->close();
        file_put_contents($f, "new1\nnew2\n");
        $s2 = new LogSource($f, false, $c);
        $s2->open();
        T::eq('truncated', $s2->resetReason());
        T::eq(['new1', 'new2'], $read($s2));
    },
    'inode change (rotation) restarts from zero' => static function () use ($dir, $read, $cp): void {
        $f = $dir() . '/r.log';
        file_put_contents($f, "a\nb\n");
        $s = new LogSource($f);
        $s->open();
        $read($s);
        $c = $cp($s);
        $s->close();
        rename($f, $f . '.old');
        file_put_contents($f . '.tmp', "c\nd\ne\nf\n");
        rename($f . '.tmp', $f);
        $s2 = new LogSource($f, false, $c);
        $s2->open();
        T::eq('inode', $s2->resetReason());
        T::eq(['c', 'd', 'e', 'f'], $read($s2));
    },
    'same inode, rewritten content larger than offset -> fingerprint restart' => static function () use ($dir, $read, $cp): void {
        $f = $dir() . '/f.log';
        file_put_contents($f, "first line of the old file\nsecond\n");
        $s = new LogSource($f);
        $s->open();
        $read($s);
        $c = $cp($s);
        $s->close();
        $fh = fopen($f, 'r+');
        ftruncate($fh, 0);
        fwrite($fh, str_repeat("REPLACED content line\n", 10));
        fclose($fh);
        $s2 = new LogSource($f, false, $c);
        $s2->open();
        T::eq('replaced', $s2->resetReason());
        T::eq(10, count($read($s2)));
    },
    'small file growing past 256 bytes keeps its checkpoint' => static function () use ($dir, $read, $cp): void {
        $f = $dir() . '/g.log';
        file_put_contents($f, "short\n");
        $s = new LogSource($f);
        $s->open();
        $read($s);
        $c = $cp($s);
        $s->close();
        file_put_contents($f, str_repeat("more more more more\n", 30), FILE_APPEND);
        $s2 = new LogSource($f, false, $c);
        $s2->open();
        T::null($s2->resetReason());
        T::eq(30, count($read($s2)));
    },
    'chunk boundaries and huge lines' => static function () use ($dir, $read): void {
        $f = $dir() . '/big.log';
        $fh = fopen($f, 'w');
        $n = 0;
        $len = 0;
        while ($len < 3 * LogSource::CHUNK) {
            $l = 'line ' . $n++ . ' ' . str_repeat('x', $n % 300);
            fwrite($fh, $l . "\n");
            $len += strlen($l) + 1;
        }
        fwrite($fh, str_repeat('Z', 3 * LogSource::CHUNK) . "\nafter\n");
        fclose($fh);
        $s = new LogSource($f);
        $s->open();
        $lines = $read($s);
        T::eq($n + 2, count($lines));
        T::eq('line 0 x', $lines[0]);
        T::eq(LogSource::MAX_LINE, strlen($lines[$n]));
        T::eq('after', $lines[$n + 1]);
        T::eq(filesize($f), $s->offset());
    },
    'gzip archives: multi-member, zlib and gzip -dc fallback' => static function () use ($dir, $read): void {
        $f = $dir() . '/arch.gz';
        file_put_contents($f, gzencode("a\nb\n") . gzencode("c\nd-no-newline"));
        foreach ([false, true] as $cli) {
            LogSource::$forceGzipCli = $cli;
            try {
                $s = new LogSource($f, true, null, true);
                $s->open();
                T::eq(['a', 'b', 'c', 'd-no-newline'], $read($s), $cli ? 'gzip -dc' : 'zlib');
                $s->close();
            } finally {
                LogSource::$forceGzipCli = false;
            }
        }
    },
    'gzip archive larger than one chunk' => static function () use ($dir, $read): void {
        $f = $dir() . '/large.gz';
        $data = '';
        for ($i = 0; $i < 60000; $i++) {
            $data .= "line $i " . md5((string) $i) . "\n";
        }
        file_put_contents($f, gzencode($data));
        $s = new LogSource($f, false, null, true);
        $s->open();
        $lines = $read($s);
        T::eq(60000, count($lines));
        T::eq('line 59999 ' . md5('59999'), $lines[59999]);
    },
    'corrupt gzip raises' => static function () use ($dir, $read): void {
        $f = $dir() . '/bad.gz';
        file_put_contents($f, "\x1f\x8b" . str_repeat('garbage', 50));
        $s = new LogSource($f, false, null, true);
        $s->open();
        T::throws(\RuntimeException::class, static fn () => $read($s));
    },
    'symlinked archive is refused' => static function () use ($dir): void {
        $t = $dir() . '/target.gz';
        file_put_contents($t, gzencode("x\n"));
        @unlink($dir() . '/link.gz');
        symlink($t, $dir() . '/link.gz');
        $s = new LogSource($dir() . '/link.gz', false, null, true);
        T::throws(\RuntimeException::class, static fn () => $s->open(), 'regular file');
    },
];
