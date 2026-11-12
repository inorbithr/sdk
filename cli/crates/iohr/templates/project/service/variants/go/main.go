// Command @@NAME@@ is the service: probes, JSON logs, traces and metrics on the standard library.
package main

import (
	"context"
	"errors"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"syscall"
	"time"
)

// config is read from the environment: the names are in .env.example.
type config struct {
	host string
	port int
}

func loadConfig(get func(string) string) (config, error) {
	c := config{host: "127.0.0.1", port: 8080}
	if v := get("HOST"); v != "" {
		c.host = v
	}
	if v := get("PORT"); v != "" {
		p, err := strconv.Atoi(v)
		if err != nil || p < 0 || p > 65535 {
			return c, errors.New("PORT must be a number from 0 to 65535")
		}
		c.port = p
	}
	return c, nil
}

// serve listens on ln and returns when ctx ends, after the requests in flight finish (10 s at most).
func serve(ctx context.Context, ln net.Listener, svc *service) error {
	srv := &http.Server{
		Handler:           svc.handler(),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       15 * time.Second,
		WriteTimeout:      30 * time.Second,
		IdleTimeout:       60 * time.Second,
	}
	errc := make(chan error, 1)
	go func() { errc <- srv.Serve(ln) }()
	svc.ready.Store(true)
	svc.log.Info("listening", slog.String("addr", ln.Addr().String()))
	select {
	case err := <-errc:
		return err
	case <-ctx.Done():
	}
	svc.ready.Store(false)
	svc.log.Info("shutting down")
	shutdown, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	return srv.Shutdown(shutdown)
}

func main() {
	log := newLogger(os.Stdout)
	cfg, err := loadConfig(os.Getenv)
	if err != nil {
		log.Error("bad configuration", slog.String("error", err.Error()))
		os.Exit(2)
	}
	ln, err := net.Listen("tcp", net.JoinHostPort(cfg.host, strconv.Itoa(cfg.port)))
	if err != nil {
		log.Error("cannot listen", slog.String("error", err.Error()))
		os.Exit(1)
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if err := serve(ctx, ln, newService(log)); err != nil {
		log.Error("stopped", slog.String("error", err.Error()))
		os.Exit(1)
	}
}
