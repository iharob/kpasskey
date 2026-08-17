# kpasskey — build and install
#
# PREFIX is where things end up (/usr by default, since these are system integration
# points: systemd units, a PAM module and a Plasma KCM all live under /usr).
# DESTDIR is the usual staging root for packaging: `make install DESTDIR=/tmp/pkg`.

PREFIX     ?= /usr
DESTDIR    ?=

BINDIR     := $(PREFIX)/bin
LIBDIR     := $(PREFIX)/lib
SYSTEMDDIR := $(LIBDIR)/systemd/system
USERDIR    := $(LIBDIR)/systemd/user
TMPFILESDIR:= $(LIBDIR)/tmpfiles.d
SECURITYDIR:= $(LIBDIR)/security
DATADIR    := $(PREFIX)/share/kpasskey

CARGO      ?= cargo
CC         ?= cc
CMAKE      ?= cmake
INSTALL    ?= install

DAEMON_BIN := daemon/target/release/kpk-spike
PAM_MODULE := pam/tests/pam_kpk_spike.so
KCM_BUILD  := kcm/build

.PHONY: all daemon pam kcm install install-daemon install-pam install-kcm clean

all: daemon pam kcm

daemon:
	$(CARGO) build --release --locked --manifest-path daemon/Cargo.toml

# A warning is a defect; this is the same gate the Rust and Kotlin halves use.
pam: $(PAM_MODULE)

$(PAM_MODULE): pam/tests/pam_kpk_spike.c
	$(CC) -shared -fPIC -O2 -Wall -Wextra -Werror -D_FORTIFY_SOURCE=3 \
	    -o $@ $< -lpam

kcm:
	$(CMAKE) -S kcm -B $(KCM_BUILD) -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=$(PREFIX)
	$(CMAKE) --build $(KCM_BUILD)

install: install-daemon install-pam install-kcm

install-daemon: daemon
	$(INSTALL) -Dm755 $(DAEMON_BIN) $(DESTDIR)$(BINDIR)/kpk-spike
	$(INSTALL) -Dm644 daemon/data/systemd/kpasskey-fido.service \
	    $(DESTDIR)$(SYSTEMDDIR)/kpasskey-fido.service
	$(INSTALL) -Dm644 daemon/data/systemd/kpasskey.service \
	    $(DESTDIR)$(USERDIR)/kpasskey.service
	$(INSTALL) -Dm644 daemon/data/tmpfiles/kpasskey.conf \
	    $(DESTDIR)$(TMPFILESDIR)/kpasskey.conf
	# The units ship pointing at the build tree so a rebuild takes effect on restart;
	# rewrite that to the installed binary.
	sed -i 's|^ExecStart=.*/target/release/kpk-spike|ExecStart=$(BINDIR)/kpk-spike|' \
	    $(DESTDIR)$(SYSTEMDDIR)/kpasskey-fido.service \
	    $(DESTDIR)$(USERDIR)/kpasskey.service

install-pam: pam
	$(INSTALL) -Dm755 $(PAM_MODULE) $(DESTDIR)$(SECURITYDIR)/pam_kpk_spike.so
	# Shipped as a reference, never installed into /etc/pam.d: silently rewriting an auth
	# stack has "cannot log in" as its failure mode. Install it yourself, with a root shell
	# already open on another TTY.
	$(INSTALL) -Dm644 pam/tests/polkit-1.spike $(DESTDIR)$(DATADIR)/polkit-1.example

install-kcm: kcm
	DESTDIR=$(DESTDIR) $(CMAKE) --install $(KCM_BUILD) --prefix $(PREFIX)

clean:
	$(CARGO) clean --manifest-path daemon/Cargo.toml
	rm -f $(PAM_MODULE)
	rm -rf $(KCM_BUILD)
