import { Button } from "./components/button";
import { Dialog } from "./components/dialog";
import { Input } from "./components/input";
import { fetchUser } from "./api/users";
import { fetchPosts } from "./api/posts";
import { fetchComments } from "./api/comments";
import { slugify } from "./utils/slugify";
import { debounce } from "./utils/debounce";
import { clamp } from "./utils/clamp";

console.log(Button, Dialog, Input, fetchUser, fetchPosts, fetchComments, slugify, debounce, clamp);
