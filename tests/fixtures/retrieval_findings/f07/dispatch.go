package fixture
 type Authorizer interface { Authorize(string) bool }
 type Routes struct { service Authorizer }
 func (r Routes) Handle(token string) bool { return r.service.Authorize(token) }
 type Allow struct{}
 func (Allow) Authorize(string) bool { return true }
 type Deny struct{}
 func (Deny) Authorize(string) bool { return false }
