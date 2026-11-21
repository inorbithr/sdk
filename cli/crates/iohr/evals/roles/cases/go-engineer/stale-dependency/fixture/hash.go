// Package accounts stores account passwords.
package accounts

import "golang.org/x/crypto/bcrypt"

// Hash returns the bcrypt hash of a password.
func Hash(password string) ([]byte, error) {
	return bcrypt.GenerateFromPassword([]byte(password), bcrypt.DefaultCost)
}

// Matches reports whether password matches hash.
func Matches(hash []byte, password string) bool {
	return bcrypt.CompareHashAndPassword(hash, []byte(password)) == nil
}
