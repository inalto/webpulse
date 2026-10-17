#!/usr/local/cpanel/3rdparty/bin/perl
#WHMADDON:webpulse:WebPulse Analytics:webpulse.png
#ACLS:all
#
# WebPulse Analytics - WHM dashboard (runs as root under cpsrvd).
#
# Renders the dashboard inside WHM's own chrome (top bar, left navigation,
# breadcrumbs): `webpulse cgi --whm --page --embedded` prints the dashboard as an
# HTML fragment, which is handed to WHM's master template through
# Cpanel::Template::process_template() and webpulse/index.tmpl. The binary derives
# the principal (administrator or reseller) from REMOTE_USER; its errors (403 for
# a user that is neither root nor a reseller owning accounts, 500) are passed
# through unchanged. Without Cpanel::Template (not a cPanel perl) the full
# standalone page is printed instead.
#
# Binary: $WEBPULSE_BIN, else /usr/local/webpulse/bin/webpulse, else bin/webpulse
# next to this script (tests). $WEBPULSE_BIN is honoured only when the file (after
# symlinks) and its directory are owned by the user running this script (root under
# cpsrvd) and are not group/other-writable, so an environment variable cannot make
# root execute somebody else's program.

use strict;
use warnings;
use lib '/usr/local/cpanel';

use Cwd ();
use File::Basename ();

# WHM's ACL state (the master template draws the navigation from it)
eval { require Whostmgr::ACLS; Whostmgr::ACLS::init_acls(); 1 };

sub trusted {
    my ($path) = @_;
    my @st = stat($path) or return 0;
    return 0 if $st[4] != $>;
    return ( $st[2] & 022 ) == 0;
}

sub find_binary {
    my $env = $ENV{WEBPULSE_BIN};
    if ( defined $env && length $env ) {
        my $real = Cwd::realpath($env);
        if ( defined $real && trusted($env) && trusted( File::Basename::dirname($real) ) && -x $env && !-d $env ) {
            return $env;
        }
    }
    my $default = '/usr/local/webpulse/bin/webpulse';
    return $default if -x $default && !-d $default;
    my $near = File::Basename::dirname($0) . '/bin/webpulse';
    return $near if -x $near && !-d $near;
    return;
}

sub unavailable {
    print "Status: 500 Internal Server Error\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\r\n";
    print '<!doctype html><html lang="en"><head><meta charset="utf-8"><title>WebPulse Analytics is not available</title></head>';
    print '<body><h2>WebPulse Analytics is not available</h2><p>The dashboard could not be loaded. Is WebPulse installed?</p></body></html>';
    exit 0;
}

# Runs the binary (no shell, the CGI environment is inherited, stdin is empty) and
# returns its whole output.
sub run_binary {
    my ( $bin, @args ) = @_;
    open( my $fh, '-|', $bin, @args ) or return;
    binmode $fh;
    local $/;
    my $out = <$fh>;
    close $fh;
    return defined $out && length $out ? $out : undef;
}

# Splits a CGI response into (status, head, body).
sub parse_cgi {
    my ($out) = @_;
    my ( $head, $body ) = split /\r?\n\r?\n/, $out, 2;
    return ( 500, '', '' ) unless defined $body;
    my $status = 200;
    for my $line ( split /\r?\n/, $head ) {
        $status = $1 if $line =~ /^Status:\s*(\d{3})\b/i;
    }
    return ( $status, $head, $body );
}

my $bin = find_binary() or unavailable();
open( STDIN, '<', '/dev/null' );
binmode STDOUT;

my $out = run_binary( $bin, 'cgi', '--whm', '--page', '--embedded', '--api-url', 'api.cgi' );
unavailable() unless defined $out;
my ( $status, undef, $body ) = parse_cgi($out);

if ( $status < 200 || $status > 299 ) {
    print $out;    # 403 / 500 pages of the binary, unchanged
    exit 0;
}

if ( eval { require Cpanel::Template; 1 } ) {
    print "Content-type: text/html\r\n\r\n";
    my $ok = eval {
        Cpanel::Template::process_template(
            'whostmgr',
            {
                template_file => 'webpulse/index.tmpl',
                print         => 1,
                data          => { body => $body },
            }
        );
        1;
    };
    print $body unless $ok;    # template failure: at least the dashboard itself
    exit 0;
}

# not cPanel's perl: the standalone page
my $page = run_binary( $bin, 'cgi', '--whm', '--page', '--api-url', 'api.cgi' );
unavailable() unless defined $page;
print $page;
exit 0;
