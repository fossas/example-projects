package main

import (
	"fmt"

	sq "github.com/Masterminds/squirrel"
)

func main() {
	users := sq.Select("*").From("users").Join("emails USING (email_id)")
	active := users.Where(sq.Eq{"deleted_at": nil})
	fmt.Printf(active.ToSql())
}
