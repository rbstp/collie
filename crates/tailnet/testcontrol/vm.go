package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"unicode"
)

func vmAuthKey(directory string) (string, error) {
	if filepath.Base(directory) != "collie-vm-test" {
		return "", fmt.Errorf("invalid VM test directory")
	}
	info, err := os.Lstat(directory)
	if err != nil || !info.IsDir() || info.Mode().Perm()&0077 != 0 || int(info.Sys().(*syscall.Stat_t).Uid) != os.Geteuid() {
		return "", fmt.Errorf("VM test directory must be private and owned")
	}
	fd, err := syscall.Open(filepath.Join(directory, "vm-tailnet.json"), syscall.O_RDONLY|syscall.O_NOFOLLOW, 0)
	if err != nil {
		return "", fmt.Errorf("cannot open private VM test marker")
	}
	file := os.NewFile(uintptr(fd), "vm-tailnet.json")
	defer file.Close()
	info, err = file.Stat()
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm()&0077 != 0 || info.Size() > 4096 || int(info.Sys().(*syscall.Stat_t).Uid) != os.Geteuid() {
		return "", fmt.Errorf("VM test marker must be private and owned")
	}
	var marker struct {
		AuthKey string `json:"auth_key"`
		RunID   string `json:"run_id"`
	}
	raw, err := io.ReadAll(io.LimitReader(file, 4097))
	if err != nil || len(raw) > 4096 {
		return "", fmt.Errorf("oversized VM test marker")
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	var trailing any
	if decoder.Decode(&marker) != nil || decoder.Decode(&trailing) != io.EOF || !strings.HasPrefix(marker.AuthKey, "test-authkey-collievm") || len(marker.AuthKey) < 40 || len(marker.AuthKey) > 96 || len(marker.RunID) != 36 || strings.IndexFunc(marker.AuthKey, func(value rune) bool {
		return value > unicode.MaxASCII || !(value >= 'a' && value <= 'z' || value >= 'A' && value <= 'Z' || value >= '0' && value <= '9' || value == '-')
	}) != -1 || strings.IndexFunc(marker.RunID, func(value rune) bool {
		return !(value >= 'a' && value <= 'f' || value >= 'A' && value <= 'F' || value >= '0' && value <= '9' || value == '-')
	}) != -1 {
		return "", fmt.Errorf("invalid VM test marker")
	}
	return marker.AuthKey, nil
}
