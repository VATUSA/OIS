#!/usr/bin/perl
# ==============================================================================
# flock-run.pl — hold a cross-worktree advisory lock, then run a command.
#
# Called by bin/with-lock.sh. Not meant to be run by hand.
#
#   perl flock-run.pl LOCKFILE TIMEOUT LABEL -- CMD [ARGS...]
#
# Why perl and not flock(1): flock(1) is Linux-only (util-linux) and is NOT
# installed on macOS by default, whereas /usr/bin/perl ships on both macOS and
# Linux. Perl's built-in flock() is the SAME kernel advisory lock, which
# gives us the one property that matters here: the kernel releases the lock the
# instant the holding process dies — SIGKILL, crash, or a box that goes down
# under a run. A killed suite therefore frees the queue immediately; there is no
# stale lockfile to reap, which a mkdir/PID-file lock would reintroduce (and a
# stale holder is exactly the failure this whole mechanism exists to avoid).
#
# The lock is held for the LIFETIME of CMD: we keep the lock filehandle open in
# THIS process and run CMD as a child, so releasing is tied to our exit. We do
# not exec CMD in place, because perl sets close-on-exec on the handle and the
# lock would drop the moment CMD started.
# ==============================================================================

use strict;
use warnings;
use Fcntl qw(:flock O_RDWR O_CREAT O_NOFOLLOW SEEK_SET);
use IO::Handle;

my $lockfile = shift @ARGV;
my $timeout  = shift @ARGV;
my $label    = shift @ARGV;
my $sep      = shift @ARGV;

if (!defined $lockfile || !defined $timeout || !defined $label
    || !defined $sep || $sep ne '--' || !@ARGV) {
    die "usage: flock-run.pl LOCKFILE TIMEOUT LABEL -- CMD [ARGS...]\n";
}
my @cmd = @ARGV;

# Status messages go to the controlling terminal when there is one, so a waiting
# run is visible even when the caller captured our stdout/stderr with $(...) —
# which the git hooks do. Falls back to STDERR when there is no tty.
my $status_fh;
if (open(my $tty, '>', '/dev/tty')) {
    $tty->autoflush(1);
    $status_fh = $tty;
} else {
    $status_fh = \*STDERR;
}
sub status { print {$status_fh} "[with-lock] ", @_, "\n"; }

# O_NOFOLLOW: the lock lives in a predictable /tmp path, and record_holder truncates
# it, so a symlink planted there must not redirect that write onto another file.
sysopen(my $lock, $lockfile, O_RDWR | O_CREAT | O_NOFOLLOW, 0664)
    or die "[with-lock] cannot open lock $lockfile: $!\n";
$lock->autoflush(1);

# Fast path: grab it without a word when nobody else holds it.
unless (flock($lock, LOCK_EX | LOCK_NB)) {
    status "'$label' busy — ", holder($lock);
    status "waiting", ($timeout > 0 ? " (timeout ${timeout}s)" : " (no timeout)"), "...";

    my $start       = time;
    my $last_report = $start;
    until (flock($lock, LOCK_EX | LOCK_NB)) {
        my $elapsed = time - $start;
        if ($timeout > 0 && $elapsed >= $timeout) {
            # An explicit code: die would exit with errno, which differs between Linux and macOS.
            print STDERR "[with-lock] gave up after ${timeout}s waiting for '$label' — "
              . holder($lock)
              . ". Override with OIS_LOCK_TIMEOUT, or OIS_NO_LOCK=1 to skip the queue.\n";
            exit 75;
        }
        # Poll at 5s, but never sleep past the deadline (so a small timeout fires
        # on time rather than overshooting by a whole poll interval).
        my $nap = 5;
        if ($timeout > 0) {
            my $remaining = $timeout - $elapsed;
            $nap = $remaining if $remaining < $nap;
        }
        $nap = 1 if $nap < 1;
        sleep $nap;
        # Re-report roughly every 30s so a long wait never looks like a hang.
        if (time - $last_report >= 30) {
            status "still waiting ", (time - $start), "s for '$label' — ", holder($lock);
            $last_report = time;
        }
    }
}

record_holder($lock);

my $child = fork();
die "[with-lock] fork failed: $!\n" unless defined $child;
if ($child == 0) {
    exec { $cmd[0] } @cmd;
    die "[with-lock] cannot exec $cmd[0]: $!\n";
}

# Forward the signals a caller (or a kill) would send, so cancelling the wrapper
# also stops the wrapped suite instead of orphaning it.
for my $sig (qw(TERM INT HUP)) {
    $SIG{$sig} = sub { kill $sig, $child; };
}

waitpid($child, 0);
my $raw = $?;

# Release happens here, when $lock is closed at exit. Mirror the child's fate in
# our own exit code so callers still read a truthful pass/fail.
if ($raw == -1) {
    exit 1;
} elsif ($raw & 127) {
    exit 128 + ($raw & 127);
} else {
    exit $raw >> 8;
}

# Stamp who we are into the lockfile so a waiter can name the holder. Opened
# without O_TRUNC (see sysopen) precisely so reading it here does not clobber a
# holder's line before we own the lock.
sub record_holder {
    my ($fh) = @_;
    seek($fh, 0, SEEK_SET);
    truncate($fh, 0);
    print {$fh} whoami(), "\n";
}

sub holder {
    my ($fh) = @_;
    seek($fh, 0, SEEK_SET);
    my $line = readline($fh);
    chomp $line if defined $line;
    return (defined $line && length $line) ? "held by $line" : "holder unknown";
}

sub whoami {
    my $wt = 'unknown';
    my $top = `git rev-parse --show-toplevel 2>/dev/null`;
    if (defined $top) {
        chomp $top;
        $wt = (split m{/}, $top)[-1] if length $top;
    }
    my @t = localtime;
    return sprintf('%s (pid %d, since %02d:%02d:%02d)', $wt, $$, $t[2], $t[1], $t[0]);
}
