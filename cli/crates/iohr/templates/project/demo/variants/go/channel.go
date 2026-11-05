package main

// Tokens from iohr's extension channel (the SDK's ADR 0012; PRD 0084 slice 1).
//
// iohr passes IOHR_EXT_TOKEN_SOCKET (a 0600 Unix socket). The program sends
// `POST /token {"scopes": [...]}` and gets `{"access_token", "expires_at"}` for scopes its
// manifest declares, and a refusal for any other. It is the only credential this project
// has: never read IOHR_TOKEN or run `iohr auth token`.

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"strings"
	"time"
)

const maxAnswer = 64 * 1024

// errRefused is what token returns when iohr refuses, or the channel cannot be reached.
var errRefused = errors.New("refused")

func refused(format string, args ...any) error {
	return fmt.Errorf("%w: %s", errRefused, fmt.Sprintf(format, args...))
}

// token returns an access token for scopes; the error says iohr's reason.
func token(scopes []string) (string, error) {
	path := strings.TrimSpace(os.Getenv("IOHR_EXT_TOKEN_SOCKET"))
	if path == "" {
		return "", refused("IOHR_EXT_TOKEN_SOCKET is not set: run this with iohr ext dev")
	}
	conn, err := net.DialTimeout("unix", path, 10*time.Second)
	if err != nil {
		return "", refused("cannot reach iohr's token channel: %v", err)
	}
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(10 * time.Second))
	body, _ := json.Marshal(map[string][]string{"scopes": scopes})
	request := fmt.Sprintf("POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-type: application/json\r\ncontent-length: %d\r\nconnection: close\r\n\r\n%s", len(body), body)
	if _, err := io.WriteString(conn, request); err != nil {
		return "", refused("cannot reach iohr's token channel: %v", err)
	}
	raw, err := io.ReadAll(io.LimitReader(conn, maxAnswer))
	if err != nil {
		return "", refused("cannot reach iohr's token channel: %v", err)
	}
	head, payload, _ := bytes.Cut(raw, []byte("\r\n\r\n"))
	fields := strings.SplitN(string(head), " ", 3)
	status := ""
	if len(fields) > 1 {
		status = fields[1]
	}
	var answer struct {
		AccessToken string `json:"access_token"`
		Error       string `json:"error"`
		Message     string `json:"message"`
	}
	_ = json.Unmarshal(payload, &answer)
	if status != "200" {
		if status == "" {
			status = "no answer"
		}
		return "", refused("%s %s: %s", status, answer.Error, answer.Message)
	}
	if answer.AccessToken == "" {
		return "", refused("iohr's answer holds no token")
	}
	return answer.AccessToken, nil
}
