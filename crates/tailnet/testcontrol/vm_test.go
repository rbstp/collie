package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestVMMarkerRequiresPrivateSyntheticIdentity(t *testing.T) {
	directory := filepath.Join(t.TempDir(), "collie-vm-test")
	if err := os.Mkdir(directory, 0700); err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(directory, "vm-tailnet.json")
	valid := `{"auth_key":"test-authkey-collievmaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","run_id":"00000000-0000-0000-0000-000000000000"}`
	for _, marker := range []string{valid + `{}`, strings.Replace(valid, "collievm", "collievé", 1), strings.Replace(valid, "test-authkey-", "tskey-", 1), strings.Repeat("x", 4097)} {
		if err := os.WriteFile(file, []byte(marker), 0600); err != nil {
			t.Fatal(err)
		}
		if _, err := vmAuthKey(directory); err == nil {
			t.Fatal("accepted invalid VM marker")
		}
	}
	if err := os.WriteFile(file, []byte(valid), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := vmAuthKey(directory); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(file, 0644); err != nil {
		t.Fatal(err)
	}
	if _, err := vmAuthKey(directory); err == nil {
		t.Fatal("accepted public marker")
	}
	if err := os.Remove(file); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(filepath.Join(directory, "other"), file); err != nil {
		t.Fatal(err)
	}
	if _, err := vmAuthKey(directory); err == nil {
		t.Fatal("accepted symlink marker")
	}
}
