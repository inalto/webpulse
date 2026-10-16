<?php

declare(strict_types=1);

namespace WebPulse\Http;

/**
 * Who is calling the API: a cPanel account (`user`), the WHM root
 * administrator (`admin`) or a reseller limited to the accounts it owns.
 */
final class Principal
{
    public const USER = 'user';
    public const ADMIN = 'admin';
    public const RESELLER = 'reseller';

    /**
     * @param string[]|null $ownedUsers accounts owned by a reseller
     */
    public function __construct(
        public readonly string $type,
        public readonly string $name,
        public readonly ?array $ownedUsers = null,
    ) {
        if (!in_array($type, [self::USER, self::ADMIN, self::RESELLER], true)) {
            throw new \InvalidArgumentException('Unknown principal type ' . $type);
        }
    }

    public function isAdmin(): bool
    {
        return $this->type === self::ADMIN;
    }

    public function isReseller(): bool
    {
        return $this->type === self::RESELLER;
    }

    public function isUser(): bool
    {
        return $this->type === self::USER;
    }

    /** Admin scope = admin or reseller. */
    public function hasAdminScope(): bool
    {
        return $this->type !== self::USER;
    }

    public function canSeeUser(string $user): bool
    {
        return match ($this->type) {
            self::ADMIN => true,
            self::USER => $user === $this->name,
            self::RESELLER => $user === $this->name || in_array($user, $this->ownedUsers ?? [], true),
        };
    }

    /** Admin, reseller (within scope) and user (own settings only). */
    public function canMutate(): bool
    {
        return true;
    }

    /** @return array{type:string,name:string} */
    public function toArray(): array
    {
        return ['type' => $this->type, 'name' => $this->name];
    }
}
