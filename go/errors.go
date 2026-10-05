package inorbit

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"
)

// Code is one of the platform's error codes (spec/problem.json). A code this version
// does not know is kept as the API wrote it.
type Code string

// The codes this version knows.
const (
	CodeBadRequest           Code = "bad_request"
	CodeFailedPrecondition   Code = "failed_precondition"
	CodeUnauthenticated      Code = "unauthenticated"
	CodeForbidden            Code = "forbidden"
	CodeNotFound             Code = "not_found"
	CodeMethodNotAllowed     Code = "method_not_allowed"
	CodeAlreadyExists        Code = "already_exists"
	CodeConflict             Code = "conflict"
	CodePayloadTooLarge      Code = "payload_too_large"
	CodeUnsupportedMediaType Code = "unsupported_media_type"
	CodeUnprocessable        Code = "unprocessable"
	CodeRateLimited          Code = "rate_limited"
	CodeQuotaExceeded        Code = "quota_exceeded"
	CodeCancelled            Code = "cancelled"
	CodeInternal             Code = "internal"
	CodeUnimplemented        Code = "unimplemented"
	CodeUnavailable          Code = "unavailable"
	CodeTimeout              Code = "timeout"
)

// Known reports whether this version of the SDK knows the code.
func (c Code) Known() bool {
	_, ok := codeStatus[c]
	return ok
}

// codeStatus is the HTTP status each known code travels with.
var codeStatus = map[Code]int{
	CodeBadRequest: 400, CodeFailedPrecondition: 400, CodeUnauthenticated: 401,
	CodeForbidden: 403, CodeNotFound: 404, CodeMethodNotAllowed: 405,
	CodeAlreadyExists: 409, CodeConflict: 409, CodePayloadTooLarge: 413,
	CodeUnsupportedMediaType: 415, CodeUnprocessable: 422, CodeRateLimited: 429, CodeQuotaExceeded: 429,
	CodeCancelled: 499, CodeInternal: 500, CodeUnimplemented: 501,
	CodeUnavailable: 503, CodeTimeout: 504,
}

// CodeForStatus is the code a plain-text answer with status stands for.
func CodeForStatus(status int) Code {
	switch status {
	case 400:
		return CodeBadRequest
	case 401:
		return CodeUnauthenticated
	case 403:
		return CodeForbidden
	case 404:
		return CodeNotFound
	case 405:
		return CodeMethodNotAllowed
	case 409:
		return CodeConflict
	case 413:
		return CodePayloadTooLarge
	case 415:
		return CodeUnsupportedMediaType
	case 422:
		return CodeUnprocessable
	case 429:
		return CodeRateLimited
	case 499:
		return CodeCancelled
	case 501:
		return CodeUnimplemented
	case 503:
		return CodeUnavailable
	case 504:
		return CodeTimeout
	}
	if status >= 500 {
		return CodeInternal
	}
	return Code("http_" + strconv.Itoa(status))
}

// The kinds of Detail.
const (
	DetailField   = "field"
	DetailInfo    = "info"
	DetailRetry   = "retry"
	DetailUnknown = "unknown"
)

// Detail is one entry of an error's details. Type says which fields mean something; a
// type this version does not know is kept in Raw with Type "unknown".
type Detail struct {
	// Type is field, info, retry or unknown.
	Type string

	// Field is the request field a field detail is about.
	Field string

	// Description says what is wrong with Field.
	Description string

	// Reason is an info detail's machine-readable reason.
	Reason string

	// Domain is an info detail's domain.
	Domain string

	// Metadata is an info detail's extra facts.
	Metadata map[string]string

	// AfterSeconds is how long a retry detail asks to wait.
	AfterSeconds int64

	// Raw is the detail as it came.
	Raw json.RawMessage
}

// UnmarshalJSON reads a detail by its type, keeping an unknown one.
func (d *Detail) UnmarshalJSON(data []byte) error {
	var wire struct {
		Type         string            `json:"type"`
		Field        string            `json:"field"`
		Description  string            `json:"description"`
		Reason       string            `json:"reason"`
		Domain       string            `json:"domain"`
		Metadata     map[string]string `json:"metadata"`
		AfterSeconds json.Number       `json:"after_seconds"`
	}
	*d = Detail{Raw: append(json.RawMessage(nil), data...)}
	if err := json.Unmarshal(data, &wire); err != nil {
		d.Type = DetailUnknown
		return nil //nolint:nilerr // an unreadable detail is kept as it came, never a failure
	}
	switch wire.Type {
	case DetailField:
		d.Type, d.Field, d.Description = wire.Type, wire.Field, wire.Description
	case DetailInfo:
		d.Type, d.Reason, d.Domain, d.Metadata = wire.Type, wire.Reason, wire.Domain, wire.Metadata
	case DetailRetry:
		d.Type = wire.Type
		d.AfterSeconds, _ = wire.AfterSeconds.Int64()
	default:
		d.Type = DetailUnknown
	}
	return nil
}

// MarshalJSON writes the detail as it came.
func (d Detail) MarshalJSON() ([]byte, error) {
	if len(d.Raw) > 0 {
		return d.Raw, nil
	}
	return json.Marshal(map[string]any{"type": d.Type})
}

// RawResponse is an HTTP answer as it came, for anything the typed result does not
// carry.
type RawResponse struct {
	// Status is the HTTP status.
	Status int

	// Header is the response headers.
	Header http.Header

	// Body is the body, at most 16 MiB.
	Body []byte

	// RequestID is the x-request-id this SDK sent.
	RequestID string

	// ServerRequestID is the request id the API answered with, if any.
	ServerRequestID string

	// Attempts is how many attempts the call took.
	Attempts int

	// IdempotencyKey is the Idempotency-Key the call was sent with, on an operation that
	// takes one: repeat the call with it to stay safe.
	IdempotencyKey string

	// IdempotencyReplayed reports that the API answered a repeat of an earlier call with
	// the same key (Idempotency-Replayed: true).
	IdempotencyReplayed bool

	// RateLimit is what the answer said about the rate limit; nil when it said nothing or
	// the rate_limit setting is off.
	RateLimit *RateLimit
}

// String describes the answer without its body or headers.
func (r *RawResponse) String() string {
	return fmt.Sprintf("HTTP %d (%d bytes, request id %s)", r.Status, len(r.Body), r.RequestID)
}

// Response is a typed answer and the raw one beside it.
type Response[T any] struct {
	// Value is the answer, typed.
	Value T

	// Raw is the answer as it came.
	Raw *RawResponse
}

var gatewayMessages = map[int]string{
	401: "the token was refused: it is missing, expired or revoked",
	403: "this credential may not call this route: its scopes or role do not allow it",
	404: "no such route or resource",
	429: "too many requests; try again shortly",
}

func gatewayMessage(status int) string {
	if m, ok := gatewayMessages[status]; ok {
		return m
	}
	if status >= 500 {
		return "the API failed to answer"
	}
	return "the request was refused"
}

func truncate(s string, max int) string {
	if utf8.RuneCountInString(s) <= max {
		return s
	}
	r := []rune(s)
	return string(r[:max]) + "…"
}

// APIError is the API's answer with the problem envelope, or a plain-text error from
// the gateway.
type APIError struct {
	// Status is the HTTP status.
	Status int

	// Code is the error code.
	Code Code

	// Problem is what the API said went wrong.
	Problem string

	// Details are the typed details.
	Details []Detail

	// Raw is the answer as it came.
	Raw *RawResponse
}

// newAPIError reads the error an answer stands for.
func newAPIError(raw *RawResponse) *APIError {
	e := &APIError{Status: raw.Status, Raw: raw}
	var wire struct {
		Code    string   `json:"code"`
		Error   string   `json:"error"`
		Details []Detail `json:"details"`
	}
	if err := json.Unmarshal(raw.Body, &wire); err == nil && (wire.Code != "" || wire.Error != "") {
		e.Code = Code(wire.Code)
		if e.Code == "" {
			e.Code = CodeForStatus(raw.Status)
		}
		e.Problem = wire.Error
		e.Details = wire.Details
	} else {
		e.Code = CodeForStatus(raw.Status)
		e.Problem = strings.TrimSpace(string(bytes.ToValidUTF8(raw.Body, []byte("?"))))
	}
	if e.Problem == "" {
		e.Problem = gatewayMessage(raw.Status)
	} else {
		e.Problem = truncate(e.Problem, 300)
	}
	return e
}

func (e *APIError) Error() string {
	id := ""
	if e.Raw != nil && e.Raw.ServerRequestID != "" {
		id = ", request id " + e.Raw.ServerRequestID
	}
	return fmt.Sprintf("%s (%s, HTTP %d%s)", e.Problem, e.Code, e.Status, id)
}

// RetryAfter is how long the API asked to wait, from a retry detail or Retry-After.
func (e *APIError) RetryAfter() (seconds int64, ok bool) {
	for _, d := range e.Details {
		if d.Type == DetailRetry {
			return d.AfterSeconds, true
		}
	}
	if e.Raw != nil {
		if d, ok := retryAfter(e.Raw.Header, time.Now()); ok {
			return int64(d.Seconds()), true
		}
	}
	return 0, false
}

// ConnectionError is an API that could not be reached: DNS, TCP, TLS, or a reset
// before an answer.
type ConnectionError struct {
	// Host is the unreachable host.
	Host string

	// Err is the cause.
	Err error

	// RequestID is the x-request-id the failed call was sent with, when it got that far.
	RequestID string

	// IdempotencyKey is the Idempotency-Key the failed call was sent with: repeat the
	// call with it to stay safe.
	IdempotencyKey string
}

func (e *ConnectionError) Error() string {
	return fmt.Sprintf("cannot reach %s: %v", e.Host, e.Err)
}

// Unwrap returns the cause.
func (e *ConnectionError) Unwrap() error { return e.Err }

// TimeoutError is an attempt, or a whole call, that ran out of time.
type TimeoutError struct {
	// Host is the host that did not answer.
	Host string

	// Seconds is the limit that ran out.
	Seconds int

	// Waiting names what the call was waiting for when its time ran out, such as "the
	// rate-limit window to reset"; empty when it waited for the API's answer.
	Waiting string

	// RequestID is the x-request-id the failed call was sent with, when it got that far.
	RequestID string

	// IdempotencyKey is the Idempotency-Key the failed call was sent with: repeat the
	// call with it to stay safe.
	IdempotencyKey string

	// total marks the call's total timeout (or the caller's deadline), which no retry
	// can outlast, as opposed to one attempt's.
	total bool
}

func (e *TimeoutError) Error() string {
	if e.Waiting != "" {
		return fmt.Sprintf("the call to %s ran out of its %d s while waiting for %s", e.Host, e.Seconds, e.Waiting)
	}
	return fmt.Sprintf("%s did not answer within %d s", e.Host, e.Seconds)
}

// AuthError is a failed token exchange, or a failed custom token provider.
type AuthError struct {
	// Message says what failed.
	Message string

	// OAuthError is the token endpoint's error, or HTTP <status>; empty for a transport
	// failure.
	OAuthError string

	// Err is the cause, if any.
	Err error
}

func (e *AuthError) Error() string { return e.Message }

// Unwrap returns the cause.
func (e *AuthError) Unwrap() error { return e.Err }

// ConfigProblem is one problem a ConfigError found (docs/config.md section 2.5).
type ConfigProblem struct {
	// Setting is the setting, by its catalogue name (timeout), or credential when no
	// credential was found.
	Setting string `json:"setting"`

	// Source is where the value came from: code, env INORBIT_TIMEOUT, file <path> [sdk];
	// empty when no source applies.
	Source string `json:"source"`

	// Message says what is wrong and what to do; it never holds a secret's value.
	Message string `json:"message"`
}

// ConfigError is a client configured in a way it cannot work with.
type ConfigError struct {
	// Message says what to change.
	Message string

	// problems is behind a pointer so ConfigError stays comparable.
	problems *[]ConfigProblem
}

func (e *ConfigError) Error() string { return e.Message }

// Problems lists every problem Load found, in the settings catalogue's order, each with
// its setting and source; empty for an error of one message.
func (e *ConfigError) Problems() []ConfigProblem {
	if e.problems == nil {
		return nil
	}
	return append([]ConfigProblem(nil), *e.problems...)
}

// configErrorOf is the error for problems, its message listing each.
func configErrorOf(problems []ConfigProblem) *ConfigError {
	if len(problems) == 1 && problems[0].Setting == "credential" {
		return &ConfigError{Message: problems[0].Message, problems: &problems}
	}
	var b strings.Builder
	n := len(problems)
	plural := "s"
	if n == 1 {
		plural = ""
	}
	fmt.Fprintf(&b, "configuration is invalid (%d problem%s):", n, plural)
	for _, p := range problems {
		lines := strings.Split(p.Message, "\n")
		from := ""
		if p.Source != "" {
			from = " (from " + p.Source + ")"
		}
		fmt.Fprintf(&b, "\n  %s: %s%s", p.Setting, lines[0], from)
		for _, l := range lines[1:] {
			b.WriteString("\n    " + l)
		}
	}
	return &ConfigError{Message: b.String(), problems: &problems}
}

// TooLargeError is an answer larger than 16 MiB, or a stream's event larger than 1 MiB.
type TooLargeError struct {
	// Event is set when one event of a stream was too large.
	Event bool
}

func (e *TooLargeError) Error() string {
	if e.Event {
		return "a stream's event is larger than 1 MiB; refusing to read it"
	}
	return "the answer is larger than 16 MiB; refusing to read it"
}

// DecodeError is an answer this version cannot read.
type DecodeError struct {
	// Reason says why.
	Reason string

	// Raw is the answer as it came.
	Raw *RawResponse
}

func (e *DecodeError) Error() string {
	return "the API answered something this version of the Go SDK cannot read: " + e.Reason
}
